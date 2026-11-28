package io.github.jfms7s.padelscore.score

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import io.github.jfms7s.padelscore.haptics.Haptics
import io.github.jfms7s.padelscore.net.Command
import io.github.jfms7s.padelscore.net.CommandRequest
import io.github.jfms7s.padelscore.net.MatchView
import io.github.jfms7s.padelscore.net.PadelApi
import io.github.jfms7s.padelscore.net.SendResult
import io.github.jfms7s.padelscore.net.Status
import io.github.jfms7s.padelscore.net.StreamItem
import io.github.jfms7s.padelscore.net.Team
import io.github.jfms7s.padelscore.net.lastEffectiveSeq
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Job
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import java.util.UUID

/** [NotSaved] is the local match's failed write; the server path never emits it. */
enum class Feedback { None, Accepted, NotSent, MatchOver, UndoConflict, NothingToUndo, NotSaved }

/** [feedbackId] changes with every new feedback, so the screen can flash the same one twice in a row. */
data class ScoreUi(
    val view: MatchView?,
    val live: Boolean,
    val sending: Boolean,
    val feedback: Feedback,
    val feedbackId: Long,
)

/**
 * The watch sends commands and shows only views the server returned (POST answer or stream frame),
 * never a local guess. One command at a time; a failed one is reported and never sent on its own.
 * The last request that ended "not sent" is kept only so that the same tap, repeated while the same
 * view is shown and within [RETRY_WINDOW_MS] of the buzz, goes out again with the same client id:
 * the server may have saved the first one after the watch gave up, and a known id returns the saved
 * answer instead of counting twice. Past the window the tap is taken as a new point (fresh id).
 * Views only move forward: one whose revision is not above the shown one is dropped.
 */
class ScoreModel(
    private val matchId: String,
    private val api: PadelApi,
    private val stream: Flow<StreamItem<MatchView>>,
    private val haptics: Haptics,
    private val newId: () -> String = { UUID.randomUUID().toString() },
    /** Monotonic milliseconds; only differences are used. */
    private val nowMs: () -> Long = { System.nanoTime() / 1_000_000 },
) : ViewModel() {
    private val _ui = MutableStateFlow(ScoreUi(null, live = false, sending = false, Feedback.None, 0))
    val ui: StateFlow<ScoreUi> = _ui.asStateFlow()

    private val _unauthorized = MutableStateFlow(false)
    val unauthorized: StateFlow<Boolean> = _unauthorized.asStateFlow()

    private var collection: Job? = null

    /** The last request that ended "not sent"; cleared by any other answer or a newer shown view. */
    private var unsent: CommandRequest? = null
    private var unsentAtMs = 0L

    /** A finished match takes no more points, but undo stays available (it may reopen the match). */
    fun point(team: Team) {
        val view = readyView() ?: return
        if (view.score.status is Status.Finished) return
        send(view, Command.Point(team))
    }

    fun undo() {
        val view = readyView() ?: return
        val seq = view.lastEffectiveSeq
        if (seq == null) {
            feedback(Feedback.NothingToUndo)
            haptics.refused()
            return
        }
        send(view, Command.Undo(seq))
    }

    /** Connects afresh (the flow is cold); a no-op while already collecting. */
    fun startStream() {
        if (collection?.isActive == true || _unauthorized.value) return
        collection = viewModelScope.launch { stream.collect(::onStream) }
    }

    fun stopStream() {
        collection?.cancel()
        collection = null
        _ui.update { it.copy(live = false) }
    }

    /** The shown view, or null when a command cannot go out now (nothing shown, one in flight, signed out). */
    private fun readyView(): MatchView? {
        val state = _ui.value
        if (state.sending || _unauthorized.value) return null
        return state.view
    }

    private fun send(view: MatchView, command: Command) {
        _ui.update { it.copy(sending = true, feedback = Feedback.None) }
        val commands = listOf(command)
        val request = unsent?.takeIf {
            it.commands == commands && it.baseRevision == view.revision && nowMs() - unsentAtMs <= RETRY_WINDOW_MS
        }
            ?: CommandRequest(newId(), batch = false, baseRevision = view.revision, commands = commands)
        viewModelScope.launch {
            // The API maps failures to results, but a bug there must not leave the screen stuck on "sending".
            val result = try {
                api.send(matchId, request)
            } catch (e: CancellationException) {
                throw e
            } catch (_: Exception) {
                SendResult.NotSent("send failed")
            }
            onResult(request, result)
        }
    }

    private fun onResult(request: CommandRequest, result: SendResult) {
        _ui.update { it.copy(sending = false) }
        unsent = if (result is SendResult.NotSent) request else null
        unsentAtMs = nowMs()
        when (result) {
            is SendResult.Applied -> {
                show(result.view)
                feedback(Feedback.Accepted)
                haptics.accepted()
            }
            is SendResult.NotSent -> {
                feedback(Feedback.NotSent)
                haptics.notSent()
            }
            // The server answered: 422 (match over, nothing to undo) and 404 (finished and purged,
            // or no longer ours) both mean this match takes no more commands; not a network problem.
            is SendResult.Rejected, SendResult.NotFound -> {
                feedback(Feedback.MatchOver)
                haptics.refused()
            }
            // Live commands only conflict on an undo whose target moved; a point is treated the same.
            is SendResult.Conflict -> {
                show(result.view)
                feedback(Feedback.UndoConflict)
                haptics.refused()
            }
            SendResult.Unauthorized -> signOut()
        }
    }

    private fun onStream(item: StreamItem<MatchView>) {
        when (item) {
            is StreamItem.Frame -> show(item.value)
            StreamItem.Live -> _ui.update { it.copy(live = true) }
            StreamItem.Offline -> _ui.update { it.copy(live = false) }
            StreamItem.Unauthorized -> signOut()
        }
    }

    private fun show(view: MatchView) = _ui.update { state ->
        val shown = state.view
        if (shown == null || view.revision > shown.revision) {
            // The score moved, so the next tap is a new point, not a retry of the unsent one.
            unsent = null
            state.copy(view = view)
        } else {
            state
        }
    }

    private fun feedback(kind: Feedback) = _ui.update { it.copy(feedback = kind, feedbackId = it.feedbackId + 1) }

    private fun signOut() {
        _unauthorized.value = true
        stopStream()
    }

    companion object {
        /**
         * How long after a "not sent" buzz the same tap still counts as its retry. Long enough to
         * look at the watch and tap again, short enough that the next rally's point (when no frame
         * arrived to move the view) is not mistaken for it and dropped.
         */
        const val RETRY_WINDOW_MS = 30_000L
    }
}
