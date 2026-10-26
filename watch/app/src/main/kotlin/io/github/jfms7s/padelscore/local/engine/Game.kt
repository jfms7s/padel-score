package io.github.jfms7s.padelscore.local.engine

import io.github.jfms7s.padelscore.net.GameScore
import io.github.jfms7s.padelscore.net.PointCall
import io.github.jfms7s.padelscore.net.Team

/** A regular game. Raw points are tracked per team; calls and deuces are derived. */
internal class Game(private val rule: DeuceRule) {
    private var a = 0
    private var b = 0
    private var wonBy: Team? = null

    /**
     * Award a point; returns the winner if this point ends the game.
     *
     * The result is latched: once a winner has been returned, later calls return
     * that same winner and change nothing.
     */
    fun point(team: Team): Team? {
        wonBy?.let { return it }
        when (team) {
            Team.A -> a++
            Team.B -> b++
        }
        wonBy = winner()
        return wonBy
    }

    val pointsPlayed: Int get() = a + b

    fun score(): GameScore = GameScore(
        a = call(a, b),
        b = call(b, a),
        deuces = maxOf(minOf(a, b) - 2, 0),
        decidingPoint = a == b && a >= 3 && nextPointWins(),
    )

    /** Lowest winning total at which a one-point lead already wins, if the rule has one. */
    private val suddenDeathTotal: Int? get() = when (rule) {
        DeuceRule.ADVANTAGE -> null
        DeuceRule.GOLDEN_POINT -> 4
        DeuceRule.STAR_POINT -> STAR_POINT_DEUCE + 3
    }

    /** For a level game at deuce: would the next point win outright? */
    private fun nextPointWins(): Boolean = suddenDeathTotal?.let { a >= it - 1 } ?: false

    private fun winner(): Team? {
        val (leader, leadPts, trailPts) = if (a >= b) Triple(Team.A, a, b) else Triple(Team.B, b, a)
        val lead = leadPts - trailPts
        val total = suddenDeathTotal
        val won = leadPts >= 4 && (lead >= 2 || (lead >= 1 && total != null && leadPts >= total))
        return if (won) leader else null
    }
}

private fun call(own: Int, other: Int): PointCall {
    if (own >= 3 && other >= 3) return if (own > other) PointCall.ADVANTAGE else PointCall.FORTY
    return when (own) {
        0 -> PointCall.LOVE
        1 -> PointCall.FIFTEEN
        2 -> PointCall.THIRTY
        else -> PointCall.FORTY
    }
}
