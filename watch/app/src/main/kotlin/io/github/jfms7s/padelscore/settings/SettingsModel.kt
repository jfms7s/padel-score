package io.github.jfms7s.padelscore.settings

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import io.github.jfms7s.padelscore.auth.TokenStore
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch

/** What went wrong with the last edit; the screen maps each to a string resource. */
enum class SettingsError { BadServer, BadAddresses, CouldNotSave }

/**
 * [settings] are the ones the app runs on. [applied] turns true once different settings are saved:
 * the app then rebuilds itself on them, so this model is done.
 */
data class SettingsState(
    val settings: ServerSettings,
    val error: SettingsError? = null,
    val saving: Boolean = false,
    val applied: Boolean = false,
)

class SettingsModel(
    private val store: SettingsStore,
    private val tokens: TokenStore,
    current: ServerSettings,
) : ViewModel() {
    private val _state = MutableStateFlow(SettingsState(current))
    val state: StateFlow<SettingsState> = _state.asStateFlow()

    /** Text from the keyboard or voice; bad input keeps the current URL and shows an error. */
    fun submitServer(text: String) {
        val url = parseServerUrl(text) ?: return fail(SettingsError.BadServer)
        val current = _state.value.settings
        save(current.copy(serverUrl = url)) { store.setServer(url) }
    }

    /** Text from the keyboard or voice; bad input keeps the current addresses and shows an error. */
    fun submitAddresses(text: String) {
        val addresses = parseAddressList(text) ?: return fail(SettingsError.BadAddresses)
        val current = _state.value.settings
        save(current.copy(fallback = addresses)) { store.setFallback(addresses) }
    }

    fun reset() {
        save(store.defaults) { store.reset() }
    }

    private fun fail(error: SettingsError) {
        _state.update { it.copy(error = error) }
    }

    private fun save(next: ServerSettings, write: suspend () -> Unit) {
        val state = _state.value
        if (state.saving || state.applied) return
        _state.value = state.copy(error = null, saving = true)
        viewModelScope.launch {
            try {
                // The token belongs to the old server: it goes first, so a failure can never leave it
                // pointed at a new one. A failed write after that only costs a sign-in.
                if (!sameOrigin(next.serverUrl, state.settings.serverUrl)) tokens.clear()
                write()
                // Saving what the app already runs on (say, a default as an override) needs no restart.
                _state.update { it.copy(saving = false, applied = next != state.settings) }
            } catch (e: CancellationException) {
                throw e
            } catch (_: Exception) {
                _state.update { it.copy(saving = false, error = SettingsError.CouldNotSave) }
            }
        }
    }
}
