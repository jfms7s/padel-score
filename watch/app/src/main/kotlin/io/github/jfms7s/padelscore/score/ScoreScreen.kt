package io.github.jfms7s.padelscore.score

import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.LinearEasing
import androidx.compose.animation.core.tween
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicText
import androidx.compose.foundation.text.TextAutoSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.BiasAlignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.RectangleShape
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.onLongClick
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.lifecycle.compose.LifecycleStartEffect
import androidx.wear.compose.material3.Button
import androidx.wear.compose.material3.CircularProgressIndicator
import androidx.wear.compose.material3.MaterialTheme
import androidx.wear.compose.material3.Text
import io.github.jfms7s.padelscore.R
import io.github.jfms7s.padelscore.net.MatchView
import io.github.jfms7s.padelscore.net.Status
import io.github.jfms7s.padelscore.net.Team
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

internal const val NOT_SENT_ICON_TAG = "not-sent-icon"
internal const val SERVE_BALL_TAG = "serve-ball"

private val TeamAColor = Color(0xFF1C5FD0)
private val TeamBColor = Color(0xFF0C8A5E)
private val BallColor = Color(0xFFD9EA2A)
private val Scrim = Color.Black.copy(alpha = 0.72f)
private val PillColor = Color.Black.copy(alpha = 0.45f)
private val HoldFill = Color.White.copy(alpha = 0.3f)

// A wrist tap near the bottom edge is easy to hit by accident, so undo needs a deliberate hold.
private const val UNDO_HOLD_MS = 600
private const val ACCEPTED_FLASH_MS = 400L
private const val NOTICE_MS = 3_000L
private val UndoBarHeight = 48.dp

/** The stream runs only while the screen is started: the screen going off stops it, wrist raise reconnects. */
@Composable
fun ScoreScreen(model: ScoreModel, onDone: () -> Unit) {
    LifecycleStartEffect(model) {
        model.startStream()
        onStopOrDispose { model.stopStream() }
    }
    val ui by model.ui.collectAsState()
    ScoreContent(ui, onPoint = model::point, onUndo = model::undo, onDone = onDone)
}

@Composable
internal fun ScoreContent(
    ui: ScoreUi,
    onPoint: (Team) -> Unit,
    onUndo: () -> Unit,
    onDone: () -> Unit,
    chip: String = stringResource(R.string.quick_match),
    onPlayAgain: (() -> Unit)? = null,
) {
    val feedback = visibleFeedback(ui)
    val screenShape = if (LocalConfiguration.current.isScreenRound) CircleShape else RectangleShape
    MaterialTheme {
        Box(Modifier.fillMaxSize().background(Color.Black)) {
            val view = ui.view
            if (view == null) {
                Text(stringResource(R.string.score_loading), Modifier.align(Alignment.Center), color = Color.White)
                return@Box
            }
            val finished = view.score.status as? Status.Finished
            Row(Modifier.fillMaxSize()) {
                Half(view, Team.A, TeamAColor, enabled = finished == null, onPoint, Modifier.weight(1f))
                Half(view, Team.B, TeamBColor, enabled = finished == null, onPoint, Modifier.weight(1f))
            }
            if (feedback == Feedback.Accepted) {
                Box(Modifier.fillMaxSize().border(4.dp, BallColor, screenShape))
            }
            TopBar(ui, view, chip, Modifier.align(Alignment.TopCenter))
            if (finished != null) {
                // Anchored just above the undo bar and growing upward, so a taller notice never pushes into it.
                FinishedBanner(
                    view,
                    finished,
                    feedback,
                    onDone,
                    onPlayAgain,
                    Modifier.align(Alignment.BottomCenter).padding(bottom = UndoBarHeight + 8.dp),
                )
            } else {
                Notice(feedback, Modifier.align(Alignment.Center).padding(horizontal = 28.dp))
            }
            // Undo stays on a finished match too: it takes back the winning point and reopens the match.
            UndoBar(onUndo, Modifier.align(Alignment.BottomCenter))
        }
    }
}

/** The model's feedback, minus the ones whose display time ran out; each new [ScoreUi.feedbackId] restarts the timer. */
@Composable
private fun visibleFeedback(ui: ScoreUi): Feedback {
    var expiredId by remember { mutableLongStateOf(-1L) }
    val id = ui.feedbackId
    val kind = ui.feedback
    LaunchedEffect(id) {
        val shownFor = when (kind) {
            Feedback.Accepted -> ACCEPTED_FLASH_MS
            Feedback.NotSent, Feedback.UndoConflict, Feedback.MatchOver, Feedback.NothingToUndo, Feedback.NotSaved ->
                NOTICE_MS
            Feedback.None -> return@LaunchedEffect
        }
        delay(shownFor)
        expiredId = id
    }
    return if (expiredId == id) Feedback.None else kind
}

@Composable
private fun Half(
    view: MatchView,
    team: Team,
    color: Color,
    enabled: Boolean,
    onPoint: (Team) -> Unit,
    modifier: Modifier,
) {
    val name = teamLabel(view.teams, team)
    val description = stringResource(R.string.score_point, name)
    Box(
        modifier
            .fillMaxHeight()
            .background(color)
            .clickable(enabled = enabled, role = Role.Button) { onPoint(team) }
            .semantics { contentDescription = description },
        contentAlignment = Alignment.Center,
    ) {
        Column(
            Modifier.padding(horizontal = 8.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
        ) {
            Text(name, color = Color.White, fontSize = 11.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
            BasicText(
                pointsLabel(view.score, team),
                style = TextStyle(
                    color = Color.White,
                    fontWeight = FontWeight.Bold,
                    fontFeatureSettings = "tnum",
                    textAlign = TextAlign.Center,
                ),
                maxLines = 1,
                autoSize = TextAutoSize.StepBased(minFontSize = 20.sp, maxFontSize = 72.sp),
            )
            Text(gamesSetsLabel(view.score, team), color = Color.White, fontSize = 13.sp, fontWeight = FontWeight.SemiBold)
        }
        if (isServing(view.score, team)) {
            // On the outer edge of the serving half, as in the mockup.
            val side = if (team == Team.A) -0.8f else 0.8f
            Box(
                Modifier
                    .align(BiasAlignment(side, 0.35f))
                    .size(12.dp)
                    .background(BallColor, CircleShape)
                    .testTag(SERVE_BALL_TAG),
            )
        }
        if (!enabled) Box(Modifier.matchParentSize().background(Color.Black.copy(alpha = 0.55f)))
    }
}

@Composable
private fun TopBar(ui: ScoreUi, view: MatchView, chip: String, modifier: Modifier) {
    Column(
        modifier.padding(top = 12.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(3.dp),
    ) {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(4.dp)) {
            Chip(chip)
            if (ui.sending) {
                val sending = stringResource(R.string.score_sending)
                CircularProgressIndicator(Modifier.size(14.dp).semantics { contentDescription = sending })
            }
        }
        if (!ui.live) Chip(stringResource(R.string.score_offline))
        if (decidingPoint(view.score)) {
            Chip(stringResource(if (goldenPoint(view)) R.string.score_golden_point else R.string.score_deciding_point), BallColor)
        }
        if (view.score.changeEnds) Chip(stringResource(R.string.score_change_ends))
    }
}

@Composable
private fun Chip(text: String, color: Color = Color.White) {
    Text(
        text,
        color = color,
        fontSize = 11.sp,
        maxLines = 1,
        modifier = Modifier.background(PillColor, CircleShape).padding(horizontal = 8.dp, vertical = 1.dp),
    )
}

private fun hasNotice(feedback: Feedback) = feedback != Feedback.None && feedback != Feedback.Accepted

@Composable
private fun Notice(feedback: Feedback, modifier: Modifier) {
    val text = when (feedback) {
        Feedback.NotSent -> R.string.score_not_sent
        Feedback.MatchOver -> R.string.score_match_over
        Feedback.UndoConflict -> R.string.score_undo_conflict
        Feedback.NothingToUndo -> R.string.score_nothing_to_undo
        Feedback.NotSaved -> R.string.local_not_saved
        Feedback.None, Feedback.Accepted -> return
    }
    Column(
        modifier
            .background(Scrim, RoundedCornerShape(16.dp))
            .padding(horizontal = 12.dp, vertical = 8.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        if (feedback == Feedback.NotSent) CrossedOutCloud(Modifier.size(28.dp).testTag(NOT_SENT_ICON_TAG))
        Text(stringResource(text), color = Color.White, fontSize = 14.sp, textAlign = TextAlign.Center)
    }
}

/** Decorative: the text next to it carries the meaning. */
@Composable
private fun CrossedOutCloud(modifier: Modifier) {
    Canvas(modifier) {
        val w = size.width
        val h = size.height
        drawCircle(Color.White, radius = w * 0.2f, center = Offset(w * 0.3f, h * 0.6f))
        drawCircle(Color.White, radius = w * 0.26f, center = Offset(w * 0.52f, h * 0.46f))
        drawCircle(Color.White, radius = w * 0.18f, center = Offset(w * 0.74f, h * 0.62f))
        drawRect(Color.White, topLeft = Offset(w * 0.3f, h * 0.6f), size = Size(w * 0.44f, h * 0.2f))
        val start = Offset(w * 0.15f, h * 0.15f)
        val end = Offset(w * 0.85f, h * 0.9f)
        drawLine(Scrim, start, end, strokeWidth = w * 0.18f, cap = StrokeCap.Round)
        drawLine(Color.White, start, end, strokeWidth = w * 0.07f, cap = StrokeCap.Round)
    }
}

/** Fires once the press has been held for [UNDO_HOLD_MS]; releasing earlier does nothing. */
@Composable
private fun UndoBar(onUndo: () -> Unit, modifier: Modifier) {
    val fill = remember { Animatable(0f) }
    val scope = rememberCoroutineScope()
    val undo by rememberUpdatedState(onUndo)
    val label = stringResource(R.string.score_undo)
    Box(
        modifier
            .fillMaxWidth()
            .height(UndoBarHeight)
            .background(Scrim)
            .drawBehind { drawRect(HoldFill, size = Size(size.width * fill.value, size.height)) }
            .pointerInput(Unit) {
                detectTapGestures(onPress = {
                    val hold = scope.launch {
                        fill.animateTo(1f, tween(UNDO_HOLD_MS, easing = LinearEasing))
                        undo()
                    }
                    tryAwaitRelease()
                    hold.cancel()
                    fill.snapTo(0f)
                })
            }
            .semantics(mergeDescendants = true) {
                role = Role.Button
                onLongClick(label) {
                    undo()
                    true
                }
            },
        contentAlignment = Alignment.Center,
    ) {
        Text(label, color = Color.White, fontSize = 13.sp)
    }
}

/** A notice (say, an undo that was not sent) takes the winner line's place for its few seconds. */
@Composable
private fun FinishedBanner(
    view: MatchView,
    finished: Status.Finished,
    feedback: Feedback,
    onDone: () -> Unit,
    onPlayAgain: (() -> Unit)?,
    modifier: Modifier,
) {
    val winner = finished.winner
    val banner = if (winner == null) {
        stringResource(R.string.score_draw)
    } else {
        stringResource(R.string.score_win, teamLabel(view.teams, winner))
    }
    Column(
        modifier.padding(horizontal = 24.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        if (hasNotice(feedback)) {
            Notice(feedback, Modifier)
        } else {
            Text(
                banner,
                color = Color.White,
                fontSize = 16.sp,
                fontWeight = FontWeight.Bold,
                textAlign = TextAlign.Center,
                modifier = Modifier.background(Scrim, RoundedCornerShape(12.dp)).padding(horizontal = 10.dp, vertical = 4.dp),
            )
        }
        if (onPlayAgain != null) {
            Button(onClick = onPlayAgain, modifier = Modifier.heightIn(min = 48.dp)) {
                Text(stringResource(R.string.local_play_again))
            }
        }
        Button(onClick = onDone, modifier = Modifier.heightIn(min = 48.dp)) {
            Text(stringResource(if (onPlayAgain != null) R.string.local_back else R.string.score_back))
        }
    }
}
