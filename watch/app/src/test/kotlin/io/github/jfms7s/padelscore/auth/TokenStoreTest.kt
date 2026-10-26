package io.github.jfms7s.padelscore.auth

import androidx.datastore.core.DataStore
import androidx.datastore.core.handlers.ReplaceFileCorruptionHandler
import androidx.datastore.preferences.core.emptyPreferences
import androidx.datastore.preferences.core.Preferences
import androidx.datastore.preferences.core.PreferenceDataStoreFactory
import androidx.datastore.preferences.core.edit
import androidx.datastore.preferences.core.stringPreferencesKey
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

private const val SERVER_A = "https://a.example.test/"
private const val SERVER_B = "https://b.example.test/"

/** A store for [origin], with [SERVER_A] as the build's default server. */
private fun tokenStore(dataStore: DataStore<Preferences>, cipher: TokenCipher, origin: String = SERVER_A) =
    DataStoreTokenStore(dataStore, cipher, origin = origin, legacyOrigin = SERVER_A)

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
        store = tokenStore(dataStore, cipher)
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
        val otherKey = tokenStore(dataStore, TestCipher())
        assertNull(otherKey.read())
        assertFalse(store.signedIn.first())
    }

    @Test
    fun cipher_security_exception_clears_and_returns_null() = runTest {
        store.write("pdl_TEST_abc123")
        // A fresh instance, as after a process restart: nothing cached, so the cipher is used.
        val restarted = tokenStore(dataStore, cipher)
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
        val restarted = tokenStore(dataStore, cipher)
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
        tokenStore(dataStore, cipher).write("pdl_TEST_new")
        assertEquals("pdl_TEST_new", store.read())
        assertEquals(1, cipher.decrypts)
    }

    @Test
    fun failed_read_caches_nothing() = runTest {
        store.write("pdl_TEST_abc123")
        val restarted = tokenStore(dataStore, cipher)
        cipher.failDecrypt = GeneralSecurityException("key permanently invalidated")
        assertNull(restarted.read())
        cipher.failDecrypt = null
        tokenStore(dataStore, cipher).write("pdl_TEST_new")
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
        val recovering = tokenStore(handled, cipher)
        assertFalse(recovering.signedIn.first())
        recovering.write("pdl_TEST_abc123")
        assertEquals("pdl_TEST_abc123", recovering.read())
    }

    @Test
    fun token_is_read_back_by_a_store_for_the_same_server() = runTest {
        store.write("pdl_TEST_abc123")
        val sameServer = tokenStore(dataStore, cipher, SERVER_A)
        assertTrue(sameServer.signedIn.first())
        assertEquals("pdl_TEST_abc123", sameServer.read())
    }

    @Test
    fun token_from_another_server_reads_as_signed_out_and_is_dropped() = runTest {
        store.write("pdl_TEST_abc123")
        val otherServer = tokenStore(dataStore, cipher, SERVER_B)
        assertFalse(otherServer.signedIn.first())
        assertNull(otherServer.read())
        // Dropped from the file, not just hidden: going back to server A doesn't bring it back.
        assertNull(tokenStore(dataStore, cipher, SERVER_A).read())
    }

    @Test
    fun late_write_by_the_old_servers_store_never_reaches_the_new_server() = runTest {
        // A sign-in still running for server A when the watch switched to B gets its token afterwards.
        val newServer = tokenStore(dataStore, cipher, SERVER_B)
        assertFalse(newServer.signedIn.first())
        tokenStore(dataStore, cipher, SERVER_A).write("pdl_TEST_for_A")
        assertFalse(newServer.signedIn.first())
        assertNull(newServer.read())
    }

    @Test
    fun save_cut_short_leaves_no_token_for_the_new_server() = runTest {
        // Signed in to A; the settings save wrote B but was swiped away before the token was cleared.
        store.write("pdl_TEST_for_A")
        val nextLaunch = tokenStore(dataStore, cipher, SERVER_B)
        assertFalse(nextLaunch.signedIn.first())
        assertNull(nextLaunch.read())
    }

    @Test
    fun legacy_token_without_origin_belongs_to_the_default_server() = runTest {
        val blob = java.util.Base64.getEncoder().encodeToString(cipher.encrypt("pdl_TEST_old".toByteArray()))
        dataStore.edit { it[stringPreferencesKey("device_token")] = blob }
        assertFalse(tokenStore(dataStore, cipher, SERVER_B).signedIn.first())
        val onDefault = tokenStore(dataStore, cipher, SERVER_A)
        assertTrue(onDefault.signedIn.first())
        assertEquals("pdl_TEST_old", onDefault.read())
    }

    @Test
    fun legacy_token_is_dropped_when_running_on_another_server() = runTest {
        val blob = java.util.Base64.getEncoder().encodeToString(cipher.encrypt("pdl_TEST_old".toByteArray()))
        dataStore.edit { it[stringPreferencesKey("device_token")] = blob }
        assertNull(tokenStore(dataStore, cipher, SERVER_B).read())
        assertNull(tokenStore(dataStore, cipher, SERVER_A).read())
    }
}
