package io.github.jfms7s.padelscore.local

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.font.FontWeight
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
import io.github.jfms7s.padelscore.net.Team
import io.github.jfms7s.padelscore.score.Feedback
import io.github.jfms7s.padelscore.score.TeamAColor
import io.github.jfms7s.padelscore.score.TeamBColor

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

/**
 * The serving team, picked on the score screen's two halves: tapping a half finishes the setup, which [onNext]
 * receives. A start that failed to save says so here, where a half can be tapped again.
 */
@Composable
fun LocalServeScreen(model: LocalMatchModel, onNext: (LocalSetup) -> Unit) {
    val draft by model.draft.collectAsState()
    val ui by model.ui.collectAsState()
    val opened = remember { model.ui.value.feedbackId }
    val notSaved = ui.feedbackSince(opened) == Feedback.NotSaved
    val pick = { team: Team -> model.pickFirstTeam(team)?.let(onNext); Unit }
    MaterialTheme {
        Box(Modifier.fillMaxSize().background(Color.Black)) {
            Row(Modifier.fillMaxSize()) {
                ServeHalf(Team.A, TeamAColor, draft.firstTeam == Team.A, pick, Modifier.weight(1f))
                ServeHalf(Team.B, TeamBColor, draft.firstTeam == Team.B, pick, Modifier.weight(1f))
            }
            Column(
                Modifier.align(Alignment.TopCenter).padding(top = 12.dp, start = 24.dp, end = 24.dp),
                horizontalAlignment = Alignment.CenterHorizontally,
            ) {
                ServeCaption(stringResource(R.string.local_serve_title))
                if (notSaved) ServeCaption(stringResource(R.string.local_not_saved))
            }
            Box(Modifier.align(Alignment.BottomCenter).padding(bottom = 12.dp, start = 24.dp, end = 24.dp)) {
                ServeCaption(stringResource(R.string.local_serve_hint))
            }
        }
    }
}

@Composable
private fun ServeHalf(team: Team, color: Color, selected: Boolean, onPick: (Team) -> Unit, modifier: Modifier) {
    val name = teamName(team)
    Box(
        modifier
            .fillMaxHeight()
            .background(color)
            .then(if (selected) Modifier.border(3.dp, Color.White).testTag(CHOICE_SELECTED_TAG) else Modifier)
            .clickable(role = Role.Button) { onPick(team) },
        contentAlignment = Alignment.Center,
    ) {
        Text(name, color = Color.White, fontSize = 20.sp, fontWeight = FontWeight.Bold)
    }
}

@Composable
private fun ServeCaption(text: String) {
    Text(
        text,
        color = Color.White,
        fontSize = 12.sp,
        textAlign = TextAlign.Center,
        modifier = Modifier.padding(vertical = 2.dp).background(Color.Black.copy(alpha = 0.45f), CircleShape)
            .padding(horizontal = 8.dp, vertical = 2.dp),
    )
}

@Composable
private fun teamName(team: Team): String =
    stringResource(if (team == Team.A) R.string.local_team_a else R.string.local_team_b)
