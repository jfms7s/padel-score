package io.github.jfms7s.padelscore.auth

import androidx.datastore.preferences.core.PreferenceDataStoreFactory
import androidx.test.platform.app.InstrumentationRegistry
import java.io.File
import java.security.KeyStore
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.runBlocking
import org.junit.After
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Before
import org.junit.Test

/** Needs a real AndroidKeyStore: run with `./gradlew :app:connectedDebugAndroidTest` on a watch or Wear emulator. */
class KeystoreTokenStoreTest {
    private val alias = "padel-device-token-test"
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private lateinit var file: File

    @Before
    fun setUp() {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        file = File(context.cacheDir, "keystore-token-test-${System.nanoTime()}.preferences_pb")
        deleteKey()
    }

    @After
    fun tearDown() {
        scope.cancel()
        file.delete()
        deleteKey()
    }

    private fun deleteKey() {
        KeyStore.getInstance("AndroidKeyStore").apply { load(null) }.deleteEntry(alias)
    }

    private fun newStore() = DataStoreTokenStore(
        PreferenceDataStoreFactory.create(scope = scope) { file },
        KeystoreCipher(alias),
        origin = "https://example.test/",
        legacyOrigin = "https://example.test/",
    )

    @Test
    fun cipher_round_trips_and_does_not_leak_plaintext() {
        val cipher = KeystoreCipher(alias)
        val plain = "pdl_TEST_abc123".toByteArray()
        val blob = cipher.encrypt(plain)
        assertFalse(String(blob, Charsets.ISO_8859_1).contains("pdl_TEST"))
        assertArrayEquals(plain, cipher.decrypt(blob))
    }

    @Test
    fun token_round_trips_through_the_real_keystore() = runBlocking {
        val store = newStore()
        store.write("pdl_TEST_abc123")
        assertEquals("pdl_TEST_abc123", store.read())
    }

    @Test
    fun lost_key_clears_and_returns_null() = runBlocking {
        val store = newStore()
        store.write("pdl_TEST_abc123")
        deleteKey()
        assertNull(store.read())
        assertNull(store.read())
    }
}
