package io.github.jfms7s.padelscore.local.engine

import io.github.jfms7s.padelscore.net.Team

/** Points needed to win a regular tie-break. */
internal const val TIEBREAK_TO = 7

/** Points needed to win a super tie-break. */
internal const val SUPER_TIEBREAK_TO = 10

/** A tie-break (first to [target], win by 2). Use [SUPER_TIEBREAK_TO] for a super tie-break. */
internal class TieBreak(private val target: Int) {
    var a = 0
        private set
    var b = 0
        private set
    private var wonBy: Team? = null

    /**
     * Award a point; returns the winner if this point ends the tie-break.
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
        wonBy = when {
            a >= target && a - b >= 2 -> Team.A
            b >= target && b - a >= 2 -> Team.B
            else -> null
        }
        return wonBy
    }

    val pointsPlayed: Int get() = a + b
}
