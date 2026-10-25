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
import kotlinx.coroutines.Job
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import java.util.UUID

enum class Feedback { None, Accepted, NotSent, MatchOver, UndoConflict }

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
 * never a local guess. One command at a time; a failed one is reported, never stored or retried.
 * Views only move forward: one whose revision is not above the shown one is dropped.
 */
class ScoreModel(
    private val matchId: String,
    private val api: PadelApi,
    private val stream: Flow<StreamItem<MatchView>>,
    private val haptics: Haptics,
    private val newId: () -> String = { UUID.randomUUID().toString() },
) : ViewModel() {
    private val _ui = MutableStateFlow(ScoreUi(null, live = false, sending = false, Feedback.None, 0))
    val ui: StateFlow<ScoreUi> = _ui.asStateFlow()

    private val _unauthorized = MutableStateFlow(false)
    val unauthorized: StateFlow<Boolean> = _unauthorized.asStateFlow()

    private var collection: Job? = null

    fun point(team: Team) = send { Command.Point(team) }

    fun undo() = send { view -> view.lastEffectiveSeq?.let { Command.Undo(it) } }

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

    /** [command] gets the shown view and returns null when there is nothing to send. */
    private fun send(command: (MatchView) -> Command?) {
        val state = _ui.value
        val view = state.view ?: return
        if (state.sending || _unauthorized.value || view.score.status is Status.Finished) return
        val next = command(view) ?: return
        _ui.value = state.copy(sending = true, feedback = Feedback.None)
        val request = CommandRequest(newId(), batch = false, baseRevision = view.revision, commands = listOf(next))
        viewModelScope.launch { onResult(api.send(matchId, request)) }
    }

    private fun onResult(result: SendResult) {
        _ui.update { it.copy(sending = false) }
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
        if (shown == null || view.revision > shown.revision) state.copy(view = view) else state
    }

    private fun feedback(kind: Feedback) = _ui.update { it.copy(feedback = kind, feedbackId = it.feedbackId + 1) }

    private fun signOut() {
        _unauthorized.value = true
        stopStream()
    }
}
