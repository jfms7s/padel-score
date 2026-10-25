package io.github.jfms7s.padelscore.score

import io.github.jfms7s.padelscore.net.Current
import io.github.jfms7s.padelscore.net.PointCall
import io.github.jfms7s.padelscore.net.Score
import io.github.jfms7s.padelscore.net.Status
import io.github.jfms7s.padelscore.net.Team
import io.github.jfms7s.padelscore.net.Teams

// Pure formatting of a server Score; nothing here computes or predicts a score.

fun pointsLabel(score: Score, team: Team): String = when (val current = score.current) {
    is Current.Game -> callLabel(if (team == Team.A) current.score.a else current.score.b)
    is Current.TieBreak -> (if (team == Team.A) current.score.a else current.score.b).toString()
    Current.None -> ""
}

private fun callLabel(call: PointCall): String = when (call) {
    PointCall.LOVE -> "0"
    PointCall.FIFTEEN -> "15"
    PointCall.THIRTY -> "30"
    PointCall.FORTY -> "40"
    PointCall.ADVANTAGE -> "AD"
}

/** "G4 S1"; a timed match has no sets, so only "G7". */
fun gamesSetsLabel(score: Score, team: Team): String {
    val games = "G${score.games.getOrElse(team.ordinal) { 0 }}"
    if (score.sets?.isEmpty() == true) return games
    return "$games S${score.setsWon.getOrElse(team.ordinal) { 0 }}"
}

fun teamLabel(teams: Teams, team: Team): String = (if (team == Team.A) teams.a else teams.b).joinToString(" / ")

fun isServing(score: Score, team: Team): Boolean = score.status == Status.InProgress && score.server.team == team

fun decidingPoint(score: Score): Boolean = (score.current as? Current.Game)?.score?.decidingPoint == true
