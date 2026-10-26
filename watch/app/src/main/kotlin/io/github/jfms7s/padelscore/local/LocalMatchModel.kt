package io.github.jfms7s.padelscore.local

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import io.github.jfms7s.padelscore.haptics.Haptics
import io.github.jfms7s.padelscore.local.engine.DecidingSet
import io.github.jfms7s.padelscore.local.engine.DeuceRule
import io.github.jfms7s.padelscore.local.engine.EngineScore
import io.github.jfms7s.padelscore.local.engine.SetRules
import io.github.jfms7s.padelscore.local.engine.SetsFormat
import io.github.jfms7s.padelscore.local.engine.Start
import io.github.jfms7s.padelscore.local.engine.replay
import io.github.jfms7s.padelscore.net.Player
import io.github.jfms7s.padelscore.net.Status
import io.github.jfms7s.padelscore.net.Team
import io.github.jfms7s.padelscore.score.Feedback
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import java.io.IOException

/** [score] is the replay of [match]; both are null with no match. [feedbackId] changes with every new feedback. */
data class LocalUi(
    val loaded: Boolean,
    val match: LocalMatch?,
    val score: EngineScore?,
    val lastSetup: LocalSetup?,
    val feedback: Feedback,
    val feedbackId: Long,
)

/**
 * [feedback] if it came after [feedbackId] [since], else none: feedback from before a screen opened (a failed
 * write elsewhere, a point before swiping away) belongs to that moment, not to the screen.
 */
fun LocalUi.feedbackSince(since: Long): Feedback = if (feedbackId == since) Feedback.None else feedback

/** The rules and first server picked so far; null until picked (or known from the last setup). */
data class SetupDraft(
    val deuce: DeuceRule?,
    val sets: SetsFormat?,
    val deciding: DecidingSet?,
    val firstServer: Player?,
) {
    fun filledFrom(last: LocalSetup): SetupDraft = SetupDraft(
        deuce = deuce ?: last.rules.deuce,
        sets = sets ?: last.rules.sets,
        deciding = deciding ?: last.rules.deciding,
        firstServer = firstServer ?: last.start.firstServer,
    )
}

/**
 * A match scored on the watch alone. Every change is written to the store before it is shown, so a
 * tap that was shown is never lost; a failed write is reported and changes nothing. Writes run one
 * at a time, each from the state the previous one left, so quick taps apply in order.
 */
class LocalMatchModel(
    private val store: LocalMatchStore,
    private val haptics: Haptics,
) : ViewModel() {
    private val _ui = MutableStateFlow(LocalUi(false, null, null, null, Feedback.None, 0))
    val ui: StateFlow<LocalUi> = _ui.asStateFlow()

    private val _draft = MutableStateFlow(SetupDraft(null, null, null, null))
    val draft: StateFlow<SetupDraft> = _draft.asStateFlow()

    private val writes = Mutex()

    // The model is the store's only writer: it loads once, and each write then updates ui itself.
    init {
        viewModelScope.launch { writes.withLock { load(store.state.first()) } }
    }

    /** A finished match takes no more points, but undo stays available (it may reopen the match). */
    fun point(team: Team) = write { ui ->
        val match = ui.match ?: return@write
        if (ui.score?.status is Status.Finished) return@write
        save(LocalMatch(match.setup, match.points + team), onSaved = ::accepted)
    }

    fun undo() = write { ui ->
        val match = ui.match ?: return@write
        if (match.points.isEmpty()) {
            feedback(Feedback.NothingToUndo)
            haptics.refused()
            return@write
        }
        save(LocalMatch(match.setup, match.points.dropLast(1)), onSaved = ::accepted)
    }

    /** An invalid setup is ignored. */
    fun start(setup: LocalSetup) = write { save(LocalMatch(setup, emptyList())) }

    /** A new match with the current one's rules and serve choice. */
    fun playAgain() = write { ui ->
        val match = ui.match ?: return@write
        save(LocalMatch(match.setup, emptyList()))
    }

    /** Deletes the match; the last setup stays. */
    fun discard() = write {
        try {
            store.clear()
        } catch (_: IOException) {
            notSaved()
            return@write
        }
        _ui.update { it.copy(match = null, score = null) }
    }

    fun pickDeuce(deuce: DeuceRule) = _draft.update { it.copy(deuce = deuce) }

    /** One set has no deciding set to pick, so it is a full set. */
    fun pickSets(sets: SetsFormat) = _draft.update {
        if (sets == SetsFormat.ONE) it.copy(sets = sets, deciding = DecidingSet.FULL_SET) else it.copy(sets = sets)
    }

    fun pickDeciding(deciding: DecidingSet) = _draft.update { it.copy(deciding = deciding) }

    fun pickFirstServer(player: Player) = _draft.update { it.copy(firstServer = player) }

    /** The finished setup, with [index] the first server of the other pair; null if a pick is missing. */
    fun pickOtherServer(index: Int): LocalSetup? {
        val d = _draft.value
        val rules = SetRules(d.deuce ?: return null, d.sets ?: return null, d.deciding ?: return null)
        val start = Start(d.firstServer ?: return null, index)
        return LocalSetup(rules, start).takeIf { it.valid }
    }

    private suspend fun load(state: LocalStoreState) {
        val match = state.match
        val score = match?.let { replay(it.setup.rules, it.setup.start, it.points) }
        if (match != null && score == null) {
            // Never written by this build's engine; dropping it is better than failing every launch.
            try {
                store.clear()
            } catch (_: IOException) {
                // Best effort: it is shown as none either way, and dropped again next launch.
            }
        }
        _ui.update {
            it.copy(loaded = true, match = match.takeIf { score != null }, score = score, lastSetup = state.lastSetup)
        }
        state.lastSetup?.takeIf { it.valid }?.let { last -> _draft.update { it.filledFrom(last) } }
    }

    /** Runs [block] under the write lock, with the state as the previous write left it. */
    private fun write(block: suspend (LocalUi) -> Unit) {
        viewModelScope.launch { writes.withLock { block(_ui.value) } }
    }

    /** Writes [match], then shows it. A match that doesn't replay is never written. */
    private suspend fun save(match: LocalMatch, onSaved: () -> Unit = {}) {
        val score = replay(match.setup.rules, match.setup.start, match.points) ?: return
        try {
            store.save(match)
        } catch (_: IOException) {
            notSaved()
            return
        }
        _ui.update { it.copy(match = match, score = score, lastSetup = match.setup) }
        onSaved()
    }

    private fun accepted() {
        feedback(Feedback.Accepted)
        haptics.accepted()
    }

    private fun notSaved() {
        feedback(Feedback.NotSaved)
        haptics.refused()
    }

    private fun feedback(kind: Feedback) = _ui.update { it.copy(feedback = kind, feedbackId = it.feedbackId + 1) }
}

private val LocalSetup.valid: Boolean get() = rules.valid && start.valid
