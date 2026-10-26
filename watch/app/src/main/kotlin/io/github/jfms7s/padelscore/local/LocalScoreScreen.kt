package io.github.jfms7s.padelscore.local

import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.res.stringResource
import io.github.jfms7s.padelscore.R
import io.github.jfms7s.padelscore.local.engine.EngineScore
import io.github.jfms7s.padelscore.local.engine.SetRules
import io.github.jfms7s.padelscore.local.engine.toWire
import io.github.jfms7s.padelscore.net.MatchView
import io.github.jfms7s.padelscore.net.Teams
import io.github.jfms7s.padelscore.net.WireJson
import io.github.jfms7s.padelscore.score.ScoreContent
import io.github.jfms7s.padelscore.score.ScoreUi
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put

/** The shared score screen over the local match: always live, nothing is ever sent. */
@Composable
fun LocalScoreScreen(model: LocalMatchModel, onBack: () -> Unit) {
    val ui by model.ui.collectAsState()
    val opened = remember { model.ui.value.feedbackId }
    val match = ui.match
    val score = ui.score
    if (match == null || score == null) return
    val view = localView(match, score, stringResource(R.string.local_team_a), stringResource(R.string.local_team_b))
    ScoreContent(
        ScoreUi(
            view = view,
            live = true,
            sending = false,
            feedback = ui.feedbackSince(opened),
            feedbackId = ui.feedbackId,
        ),
        onPoint = model::point,
        onUndo = model::undo,
        onDone = {
            model.discard()
            onBack()
        },
        chip = stringResource(R.string.local_match),
        onPlayAgain = model::playAgain,
    )
}

/** The match in the server's shape; `format` carries the rules so the screen can read the deuce rule. */
internal fun localView(match: LocalMatch, score: EngineScore, teamA: String, teamB: String): MatchView =
    MatchView(
        id = "local",
        format = buildJsonObject {
            put("sets", WireJson.encodeToJsonElement(SetRules.serializer(), match.setup.rules))
        },
        teams = Teams(listOf(teamA), listOf(teamB)),
        revision = match.points.size.toLong(),
        headSeq = match.points.size.toLong(),
        events = emptyList(),
        score = score.toWire(),
    )
