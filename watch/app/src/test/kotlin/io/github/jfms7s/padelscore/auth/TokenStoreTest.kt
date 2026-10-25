package io.github.jfms7s.padelscore.auth

import androidx.datastore.core.DataStore
import androidx.datastore.core.handlers.ReplaceFileCorruptionHandler
import androidx.datastore.preferences.core.emptyPreferences
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
    var decrypts = 0

    override fun encrypt(plain: ByteArray): ByteArray {
        val iv = ByteArray(12).also(SecureRandom()::nextBytes)
        val cipher = Cipher.getInstance("AES/GCM/NoPadding")
        cipher.init(Cipher.ENCRYPT_MODE, key, GCMParameterSpec(128, iv))
        return iv + cipher.doFinal(plain)
    }

    override fun decrypt(blob: ByteArray): ByteArray {
        decrypts++
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
        // A fresh instance, as after a process restart: nothing cached, so the cipher is used.
        val restarted = DataStoreTokenStore(dataStore, cipher)
        cipher.failDecrypt = GeneralSecurityException("key permanently invalidated")
        assertNull(restarted.read())
        assertFalse(restarted.signedIn.first())
        cipher.failDecrypt = null
        assertNull(restarted.read())
    }

    @Test
    fun read_after_write_is_served_from_memory() = runTest {
        store.write("pdl_TEST_abc123")
        assertEquals("pdl_TEST_abc123", store.read())
        assertEquals("pdl_TEST_abc123", store.read())
        assertEquals(0, cipher.decrypts)
    }

    @Test
    fun first_read_decrypts_once_then_caches() = runTest {
        store.write("pdl_TEST_abc123")
        val restarted = DataStoreTokenStore(dataStore, cipher)
        assertEquals("pdl_TEST_abc123", restarted.read())
        assertEquals("pdl_TEST_abc123", restarted.read())
        assertEquals(1, cipher.decrypts)
    }

    @Test
    fun clear_drops_the_cached_token() = runTest {
        store.write("pdl_TEST_abc123")
        assertEquals("pdl_TEST_abc123", store.read())
        store.clear()
        // Another writer puts a new token in the file: a stale cache would still answer with the old one.
        DataStoreTokenStore(dataStore, cipher).write("pdl_TEST_new")
        assertEquals("pdl_TEST_new", store.read())
        assertEquals(1, cipher.decrypts)
    }

    @Test
    fun failed_read_caches_nothing() = runTest {
        store.write("pdl_TEST_abc123")
        val restarted = DataStoreTokenStore(dataStore, cipher)
        cipher.failDecrypt = GeneralSecurityException("key permanently invalidated")
        assertNull(restarted.read())
        cipher.failDecrypt = null
        DataStoreTokenStore(dataStore, cipher).write("pdl_TEST_new")
        assertEquals("pdl_TEST_new", restarted.read())
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

    @Test
    fun corrupt_file_reads_as_null() = runTest {
        file.writeBytes(byteArrayOf(0x0F, 0x7A, 0x01, 0x55, 0x33))
        assertNull(store.read())
    }

    @Test
    fun corruption_handler_makes_signed_in_false_and_allows_new_write() = runTest {
        file.writeBytes(byteArrayOf(0x0F, 0x7A, 0x01, 0x55, 0x33))
        val handled = PreferenceDataStoreFactory.create(
            corruptionHandler = ReplaceFileCorruptionHandler { emptyPreferences() },
            scope = scope,
        ) { file }
        val recovering = DataStoreTokenStore(handled, cipher)
        assertFalse(recovering.signedIn.first())
        recovering.write("pdl_TEST_abc123")
        assertEquals("pdl_TEST_abc123", recovering.read())
    }
}
