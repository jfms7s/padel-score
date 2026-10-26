package io.github.jfms7s.padelscore.local.engine

import io.github.jfms7s.padelscore.net.Player

/** Points between ends changes in a tie-break or super tie-break. */
private const val TIEBREAK_CHANGE_EVERY = 6

/**
 * The service rotation for [start]: first, the receiving team's chosen first server, first's partner,
 * the receiving team's other player. Each pair chooses its own first server (FIP).
 */
internal fun serveOrder(start: Start): List<Player> {
    val first = start.firstServer
    val receiving = first.team.other()
    return listOf(
        first,
        Player(receiving, start.otherFirstServer),
        Player(first.team, 1 - first.index),
        Player(receiving, 1 - start.otherFirstServer),
    )
}

/** The player [steps] places along the rotation of [start]. */
internal fun inRotation(start: Start, steps: Int): Player = serveOrder(start)[steps % 4]

/**
 * Server of a tie-break after [pointsPlayed] points, when [rotation] is the
 * tie-break's first server: one point, then two each, in rotation order.
 */
internal fun tiebreakServer(start: Start, rotation: Int, pointsPlayed: Int): Player =
    inRotation(start, rotation % 4 + ((pointsPlayed + 1) / 2) % 4)

/**
 * Serve from the right after an even number of points, from the left after an odd one;
 * the receivers choose on a deciding (golden or star) point.
 */
internal fun serveSide(pointsPlayed: Int, decidingPoint: Boolean): ServeSide = when {
    decidingPoint -> ServeSide.RECEIVER_CHOOSES
    pointsPlayed % 2 == 0 -> ServeSide.RIGHT
    else -> ServeSide.LEFT
}

/**
 * Ends change after a game when the games played in its set add up to an odd number
 * (FIP Rule 5.1). A set's tie-break counts as its 13th game.
 */
internal fun endsChangeAfterGame(gamesInSet: Int): Boolean = gamesInSet % 2 == 1

/** Ends change in a tie-break (or super tie-break) after every 6 points. */
internal fun endsChangeInTiebreak(pointsPlayed: Int): Boolean =
    pointsPlayed > 0 && pointsPlayed % TIEBREAK_CHANGE_EVERY == 0
