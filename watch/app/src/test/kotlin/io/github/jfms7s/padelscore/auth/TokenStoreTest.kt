package io.github.jfms7s.padelscore.auth

import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.Preferences
import androidx.datastore.preferences.core.PreferenceDataStoreFactory
import java.io.File
import java.nio.file.Files
import java.security.GeneralSecurityException
import java.security.SecureRandom
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.test.runTest
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test

/** Same blob layout as KeystoreCipher (iv || ciphertext+tag) but with an in-memory key. */
private class TestCipher(private val key: SecretKey = newKey()) : TokenCipher {
    var failDecrypt: Exception? = null

    override fun encrypt(plain: ByteArray): ByteArray {
        val iv = ByteArray(12).also(SecureRandom()::nextBytes)
        val cipher = Cipher.getInstance("AES/GCM/NoPadding")
        cipher.init(Cipher.ENCRYPT_MODE, key, GCMParameterSpec(128, iv))
        return iv + cipher.doFinal(plain)
    }

    override fun decrypt(blob: ByteArray): ByteArray {
        failDecrypt?.let { throw it }
        val cipher = Cipher.getInstance("AES/GCM/NoPadding")
        cipher.init(Cipher.DECRYPT_MODE, key, GCMParameterSpec(128, blob.copyOfRange(0, 12)))
        return cipher.doFinal(blob, 12, blob.size - 12)
    }

    companion object {
        fun newKey(): SecretKey = KeyGenerator.getInstance("AES").apply { init(256) }.generateKey()
    }
}

class TokenStoreTest {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Unconfined)
    private lateinit var dir: File
    private lateinit var file: File
    private val cipher = TestCipher()
    private lateinit var dataStore: DataStore<Preferences>
    private lateinit var store: DataStoreTokenStore

    @Before
    fun setUp() {
        dir = Files.createTempDirectory("tokenstore").toFile()
        file = File(dir, "token.preferences_pb")
        dataStore = PreferenceDataStoreFactory.create(scope = scope) { file }
        store = DataStoreTokenStore(dataStore, cipher)
    }

    @After
    fun tearDown() {
        scope.cancel()
        dir.deleteRecursively()
    }

    @Test
    fun write_then_read_round_trips() = runTest {
        store.write("pdl_TEST_abc123")
        assertEquals("pdl_TEST_abc123", store.read())
    }

    @Test
    fun read_with_nothing_stored_is_null() = runTest {
        assertNull(store.read())
    }

    @Test
    fun stored_bytes_do_not_contain_token() = runTest {
        store.write("pdl_TEST_abc123")
        assertTrue(file.length() > 0)
        assertFalse(String(file.readBytes(), Charsets.ISO_8859_1).contains("pdl_TEST"))
    }

    @Test
    fun undecryptable_blob_clears_and_returns_null() = runTest {
        store.write("pdl_TEST_abc123")
        val otherKey = DataStoreTokenStore(dataStore, TestCipher())
        assertNull(otherKey.read())
        assertFalse(store.signedIn.first())
    }

    @Test
    fun cipher_security_exception_clears_and_returns_null() = runTest {
        store.write("pdl_TEST_abc123")
        cipher.failDecrypt = GeneralSecurityException("key permanently invalidated")
        assertNull(store.read())
        assertFalse(store.signedIn.first())
        cipher.failDecrypt = null
        assertNull(store.read())
    }

    @Test
    fun clear_emits_signed_out() = runTest {
        assertFalse(store.signedIn.first())
        store.write("pdl_TEST_abc123")
        assertTrue(store.signedIn.first())
        store.clear()
        assertFalse(store.signedIn.first())
        assertNull(store.read())
    }
}
