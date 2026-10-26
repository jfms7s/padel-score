package io.github.jfms7s.padelscore.local

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.TextUnit
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.wear.compose.foundation.lazy.TransformingLazyColumn
import androidx.wear.compose.foundation.lazy.itemsIndexed
import androidx.wear.compose.foundation.lazy.rememberTransformingLazyColumnState
import androidx.wear.compose.material3.Button
import androidx.wear.compose.material3.ButtonDefaults
import androidx.wear.compose.material3.MaterialTheme
import androidx.wear.compose.material3.ScreenScaffold
import androidx.wear.compose.material3.Text
import io.github.jfms7s.padelscore.R
import io.github.jfms7s.padelscore.local.engine.DecidingSet
import io.github.jfms7s.padelscore.local.engine.DeuceRule
import io.github.jfms7s.padelscore.local.engine.SetsFormat
import io.github.jfms7s.padelscore.net.Player
import io.github.jfms7s.padelscore.net.Team
import io.github.jfms7s.padelscore.score.Feedback

internal const val CHOICE_SELECTED_TAG = "choice-selected"

private val MinTarget = 48.dp

/**
 * One rule-picking screen: a title, any [notice] under it, a full-width button per option (the [highlighted] one
 * filled), then the hint.
 */
@Composable
internal fun ChoiceScreen(
    title: String,
    hint: String?,
    options: List<String>,
    highlighted: Int?,
    onPick: (Int) -> Unit,
    notice: String? = null,
) {
    MaterialTheme {
        val listState = rememberTransformingLazyColumnState()
        ScreenScaffold(scrollState = listState) { padding ->
            TransformingLazyColumn(state = listState, contentPadding = padding, modifier = Modifier.fillMaxSize()) {
                item { Caption(title, 15.sp) }
                if (notice != null) item { Caption(notice, 13.sp) }
                itemsIndexed(options) { index, option ->
                    val selected = index == highlighted
                    Button(
                        onClick = { onPick(index) },
                        colors = if (selected) ButtonDefaults.buttonColors() else ButtonDefaults.filledTonalButtonColors(),
                        modifier = Modifier.fillMaxWidth().heightIn(min = MinTarget).then(
                            if (selected) Modifier.testTag(CHOICE_SELECTED_TAG) else Modifier,
                        ),
                    ) { Text(option) }
                }
                if (hint != null) item { Caption(hint, 13.sp) }
            }
        }
    }
}

@Composable
private fun Caption(text: String, size: TextUnit) {
    Box(Modifier.fillMaxWidth().padding(vertical = 8.dp), contentAlignment = Alignment.Center) {
        Text(text, color = Color.White, fontSize = size, textAlign = TextAlign.Center)
    }
}

@Composable
fun LocalDeuceScreen(model: LocalMatchModel, onNext: () -> Unit) {
    val draft by model.draft.collectAsState()
    val rules = DeuceRule.entries
    ChoiceScreen(
        title = stringResource(R.string.local_deuce_title),
        hint = null,
        options = listOf(
            stringResource(R.string.local_deuce_advantage),
            stringResource(R.string.local_deuce_golden),
            stringResource(R.string.local_deuce_star),
        ),
        highlighted = draft.deuce?.let(rules::indexOf),
        onPick = { model.pickDeuce(rules[it]); onNext() },
    )
}

/** Reports the format picked, so navigation can skip the deciding screen for one set. */
@Composable
fun LocalSetsScreen(model: LocalMatchModel, onNext: (SetsFormat) -> Unit) {
    val draft by model.draft.collectAsState()
    val formats = SetsFormat.entries
    ChoiceScreen(
        title = stringResource(R.string.local_sets_title),
        hint = null,
        options = listOf(stringResource(R.string.local_sets_one), stringResource(R.string.local_sets_best_of_3)),
        highlighted = draft.sets?.let(formats::indexOf),
        onPick = { model.pickSets(formats[it]); onNext(formats[it]) },
    )
}

@Composable
fun LocalDecidingScreen(model: LocalMatchModel, onNext: () -> Unit) {
    val draft by model.draft.collectAsState()
    val sets = DecidingSet.entries
    ChoiceScreen(
        title = stringResource(R.string.local_deciding_title),
        hint = null,
        options = listOf(stringResource(R.string.local_deciding_full), stringResource(R.string.local_deciding_super)),
        highlighted = draft.deciding?.let(sets::indexOf),
        onPick = { model.pickDeciding(sets[it]); onNext() },
    )
}

@Composable
fun LocalServeScreen(model: LocalMatchModel, onNext: () -> Unit) {
    val draft by model.draft.collectAsState()
    val players = listOf(Player(Team.A, 0), Player(Team.A, 1), Player(Team.B, 0), Player(Team.B, 1))
    ChoiceScreen(
        title = stringResource(R.string.local_serve_title),
        hint = stringResource(R.string.local_serve_hint),
        options = players.map { playerName(it) },
        highlighted = draft.firstServer?.let(players::indexOf),
        onPick = { model.pickFirstServer(players[it]); onNext() },
    )
}

/**
 * The other pair's two players; a pick finishes the setup, which [onNext] receives. A start that failed to save
 * says so here, where the pick can be tapped again.
 */
@Composable
fun LocalOtherServerScreen(model: LocalMatchModel, onNext: (LocalSetup) -> Unit) {
    val draft by model.draft.collectAsState()
    val ui by model.ui.collectAsState()
    val opened = remember { model.ui.value.feedbackId }
    val notSaved = ui.feedbackSince(opened) == Feedback.NotSaved
    val otherTeam = if (draft.firstServer?.team == Team.A) Team.B else Team.A
    val players = listOf(Player(otherTeam, 0), Player(otherTeam, 1))
    // The last pick counts only while the serving pair is the same one it was then.
    val last = ui.lastSetup?.start?.takeIf { it.firstServer.team == draft.firstServer?.team }
    ChoiceScreen(
        title = stringResource(R.string.local_other_title),
        hint = null,
        options = players.map { playerName(it) },
        highlighted = last?.otherFirstServer,
        onPick = { index -> model.pickOtherServer(index)?.let(onNext) },
        notice = if (notSaved) stringResource(R.string.local_not_saved) else null,
    )
}

@Composable
private fun playerName(player: Player): String = stringResource(
    R.string.local_player,
    stringResource(if (player.team == Team.A) R.string.local_team_a else R.string.local_team_b),
    player.index + 1,
)
