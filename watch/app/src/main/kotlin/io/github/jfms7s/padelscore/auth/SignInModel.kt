package io.github.jfms7s.padelscore.auth

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import io.github.jfms7s.padelscore.net.ApiResult
import io.github.jfms7s.padelscore.net.DeviceStart
import io.github.jfms7s.padelscore.net.PadelApi
import io.github.jfms7s.padelscore.net.TokenPoll
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import okhttp3.HttpUrl

/** Why sign-in could not continue; the screen maps each to a string resource so no text (or exception) leaks from here. */
enum class FailKind { TooManyTries, Unreachable, CouldNotSave }

sealed interface SignInState {
    data object Starting : SignInState

    /** `offline` is set while the last poll could not reach the server; the code stays valid. */
    data class ShowCode(
        val userCode: String,
        val linkUrl: String,
        val secondsLeft: Int,
        val offline: Boolean = false,
    ) : SignInState

    data object Expired : SignInState
    data object Denied : SignInState
    data class Failed(val kind: FailKind) : SignInState
    data object SignedIn : SignInState
}

/** The device name sent at sign-in: the watch model, never blank. Server limit is 40 chars. */
fun deviceNameOf(model: String?): String =
    model?.trim()?.take(DEVICE_NAME_MAX)?.trim().takeUnless { it.isNullOrEmpty() } ?: DEFAULT_DEVICE_NAME

private const val DEVICE_NAME_MAX = 40
private const val DEFAULT_DEVICE_NAME = "Wear OS watch"

/** RFC 8628 §3.5: after slow_down, every later poll interval grows by 5 s. */
private const val SLOW_DOWN_STEP_SECONDS = 5

// Past the code's lifetime anyway; keeps the Int sum below from overflowing on a hostile Retry-After.
private const val MAX_WAIT_SECONDS = 3_600L

class SignInModel(
    private val api: PadelApi,
    private val tokens: TokenStore,
    private val serverUrl: HttpUrl,
    private val deviceName: String,
) : ViewModel() {
    private val _state = MutableStateFlow<SignInState>(SignInState.Starting)
    val state: StateFlow<SignInState> = _state.asStateFlow()

    private var flow: Job? = null

    init {
        restart()
    }

    fun restart() {
        flow?.cancel()
        _state.value = SignInState.Starting
        flow = viewModelScope.launch { run() }
    }

    private suspend fun run() {
        val start = when (val result = api.startDevice(deviceName)) {
            is ApiResult.Ok -> result.value
            is ApiResult.TooMany -> return fail(FailKind.TooManyTries)
            else -> return fail(FailKind.Unreachable)
        }
        poll(start)
    }

    // One-second ticks drive both the countdown and the poll schedule, so polls can never come
    // sooner than `interval` (the server answers slow_down below interval - 1 s).
    private suspend fun poll(start: DeviceStart) {
        val linkUrl = displayLink(start.verificationUri)
        var interval = start.interval
        var elapsed = 0
        var nextPoll = interval
        var offline = false
        while (true) {
            _state.value = SignInState.ShowCode(start.userCode, linkUrl, start.expiresIn - elapsed, offline)
            delay(1_000)
            elapsed++
            if (elapsed >= start.expiresIn) {
                return finish(SignInState.Expired)
            }
            if (elapsed < nextPoll) continue
            var wait = interval
            when (val poll = api.pollToken(start.deviceCode)) {
                is TokenPoll.Granted -> return save(poll.token)
                TokenPoll.Expired -> return finish(SignInState.Expired)
                TokenPoll.Denied -> return finish(SignInState.Denied)
                TokenPoll.Pending -> offline = false
                TokenPoll.SlowDown -> {
                    interval += SLOW_DOWN_STEP_SECONDS
                    wait = interval
                    offline = false
                }
                is TokenPoll.TooMany -> {
                    // Back off but keep the code: the user may be mid-way through approving it.
                    wait = maxOf(interval.toLong(), poll.retryAfterSeconds ?: 0L).coerceAtMost(MAX_WAIT_SECONDS).toInt()
                    offline = false
                }
                is TokenPoll.Failed -> offline = true
            }
            nextPoll = elapsed + wait
        }
    }

    private suspend fun save(token: String) {
        try {
            tokens.write(token)
            _state.value = SignInState.SignedIn
        } catch (e: CancellationException) {
            throw e
        } catch (_: Exception) {
            // Deliberately dropped: the exception text could carry token material.
            fail(FailKind.CouldNotSave)
        }
    }

    private fun fail(kind: FailKind) = finish(SignInState.Failed(kind))

    private fun finish(terminal: SignInState) {
        _state.value = terminal
    }

    /** `/link` against the server URL, shown without the scheme. */
    private fun displayLink(verificationUri: String): String =
        (serverUrl.resolve(verificationUri)?.toString() ?: verificationUri)
            .removePrefix("https://")
            .removePrefix("http://")
}
