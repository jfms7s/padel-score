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
 * [settings] are the ones the app runs on, null before a server is set. [applied] turns true once
 * different settings are saved: the app then rebuilds itself on them, so this model is done.
 */
data class SettingsState(
    val settings: ServerSettings?,
    val error: SettingsError? = null,
    val saving: Boolean = false,
    val applied: Boolean = false,
)

/** [tokens] is the current server's sign-in; null on a first run, when there is no server yet. */
class SettingsModel(
    private val store: SettingsStore,
    current: ServerSettings?,
    private val tokens: TokenStore?,
) : ViewModel() {
    private val _state = MutableStateFlow(SettingsState(current))
    val state: StateFlow<SettingsState> = _state.asStateFlow()

    /** Text from the keyboard or voice; bad input keeps the current URL and shows an error. */
    fun submitServer(text: String) {
        val url = parseServerUrl(text) ?: return fail(SettingsError.BadServer)
        val current = _state.value.settings
        // LAN addresses belong to a host: a new host starts with none (the owner can add them next).
        val fallback = if (current != null && url.host == current.serverUrl.host) current.fallback else emptyList()
        save(ServerSettings(url, fallback)) { store.setServer(url, fallback) }
    }

    /**
     * Text from the keyboard or voice; bad input keeps the current addresses and shows an error.
     * Before a server is set there is nothing to attach them to, so it does nothing.
     */
    fun submitAddresses(text: String) {
        val current = _state.value.settings ?: return
        val addresses = parseAddressList(text) ?: return fail(SettingsError.BadAddresses)
        save(current.copy(fallback = addresses)) { store.setFallback(addresses) }
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
                val old = state.settings
                if (old != null && !sameOrigin(next.serverUrl, old.serverUrl)) tokens?.clear()
                write()
                // Saving what the app already runs on needs no restart.
                _state.update { it.copy(settings = next, saving = false, applied = next != state.settings) }
            } catch (e: CancellationException) {
                throw e
            } catch (_: Exception) {
                _state.update { it.copy(saving = false, error = SettingsError.CouldNotSave) }
            }
        }
    }
}
