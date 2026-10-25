package io.github.jfms7s.padelscore.auth

import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.Preferences
import androidx.datastore.preferences.core.edit
import androidx.datastore.preferences.core.stringPreferencesKey
import java.security.KeyStore
import java.util.Base64
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.map

interface TokenStore {
    /** Never throws (other than cancellation): an unreadable token is cleared and reported as null. */
    suspend fun read(): String?

    /** Throws if the token cannot be encrypted or persisted; the sign-in flow must surface that. */
    suspend fun write(token: String)

    suspend fun clear()

    val signedIn: Flow<Boolean>
}

/** `blob` is `iv (12 bytes) || ciphertext+tag`. */
interface TokenCipher {
    fun encrypt(plain: ByteArray): ByteArray

    fun decrypt(blob: ByteArray): ByteArray
}

/** AES-256-GCM with a non-exportable AndroidKeyStore key, created on first use. */
class KeystoreCipher(private val alias: String = "padel-device-token") : TokenCipher {
    override fun encrypt(plain: ByteArray): ByteArray {
        val cipher = Cipher.getInstance(TRANSFORMATION)
        cipher.init(Cipher.ENCRYPT_MODE, key())
        // The Keystore generates the IV; a caller-supplied one is rejected for randomized encryption.
        return cipher.iv + cipher.doFinal(plain)
    }

    override fun decrypt(blob: ByteArray): ByteArray {
        require(blob.size > IV_BYTES) { "blob too short" }
        val cipher = Cipher.getInstance(TRANSFORMATION)
        cipher.init(Cipher.DECRYPT_MODE, key(), GCMParameterSpec(TAG_BITS, blob, 0, IV_BYTES))
        return cipher.doFinal(blob, IV_BYTES, blob.size - IV_BYTES)
    }

    @Synchronized
    private fun key(): SecretKey {
        val store = KeyStore.getInstance(PROVIDER).apply { load(null) }
        (store.getKey(alias, null) as? SecretKey)?.let { return it }
        val spec = KeyGenParameterSpec.Builder(
            alias,
            KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT,
        )
            .setKeySize(256)
            .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
            .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
            .build()
        return KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, PROVIDER)
            .apply { init(spec) }
            .generateKey()
    }

    private companion object {
        const val PROVIDER = "AndroidKeyStore"
        const val TRANSFORMATION = "AES/GCM/NoPadding"
        const val IV_BYTES = 12
        const val TAG_BITS = 128
    }
}

/**
 * The DataStore must be a single instance per process (DataStore enforces one active
 * instance per file), so build it with `preferencesDataStore` or a process-wide singleton,
 * with `ReplaceFileCorruptionHandler { emptyPreferences() }` so a corrupt file reads as signed out.
 */
class DataStoreTokenStore(
    private val dataStore: DataStore<Preferences>,
    private val cipher: TokenCipher,
) : TokenStore {
    override suspend fun read(): String? =
        try {
            val encoded = dataStore.data.map { it[KEY] }.first()
            encoded?.let { String(cipher.decrypt(Base64.getDecoder().decode(it)), Charsets.UTF_8) }
        } catch (e: CancellationException) {
            throw e
        } catch (_: Exception) {
            // Key lost after a reset, corrupt data or file, bad Base64: sign in again. Never log the blob.
            discard()
            null
        }

    /** Best effort: read() must stay total even when the file itself is unusable. */
    private suspend fun discard() {
        try {
            clear()
        } catch (e: CancellationException) {
            throw e
        } catch (_: Exception) {
        }
    }

    override suspend fun write(token: String) {
        val blob = Base64.getEncoder().encodeToString(cipher.encrypt(token.toByteArray(Charsets.UTF_8)))
        dataStore.edit { it[KEY] = blob }
    }

    override suspend fun clear() {
        dataStore.edit { it.remove(KEY) }
    }

    override val signedIn: Flow<Boolean> = dataStore.data.map { it[KEY] != null }

    private companion object {
        val KEY = stringPreferencesKey("device_token")
    }
}
