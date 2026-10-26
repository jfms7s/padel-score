package io.github.jfms7s.padelscore.local.engine

import io.github.jfms7s.padelscore.net.Current
import io.github.jfms7s.padelscore.net.Status
import io.github.jfms7s.padelscore.net.Team
import io.github.jfms7s.padelscore.net.TieBreakScore

/** Games needed to win a regular set (with a 2-game lead, or 7-6 via a tie-break). */
private const val SET_GAMES = 6

/** Who has won a regular set at [gamesA]-[gamesB], if anyone: at least 6 games with a lead of 2, or 7-6. */
private fun setWon(gamesA: Int, gamesB: Int): Team? {
    fun won(own: Int, other: Int) = own >= SET_GAMES && (own - other >= 2 || (own == SET_GAMES + 1 && other == SET_GAMES))
    return when {
        won(gamesA, gamesB) -> Team.A
        won(gamesB, gamesA) -> Team.B
        else -> null
    }
}

/** What is being played in a sets-format match. */
private sealed interface Playing {
    class InGame(val game: Game) : Playing
    class InTieBreak(val tb: TieBreak, val superTiebreak: Boolean) : Playing
    data object Over : Playing
}

/** A sets-format match in progress. Feed it points with [apply]; read it with [score]. */
class SetsMatch private constructor(private val rules: SetRules, private val start: Start) {
    private val finished = mutableListOf<SetScore>()
    private val setsWon = IntArray(2)

    /** Games in the current set (the final set's once the match is over). */
    private val games = IntArray(2)
    private var playing: Playing = Playing.Over
    private var winner: Team? = null

    /** Places along the service rotation (mod 4) of the current game's server, or of the current tie-break's first server. */
    private var rotation = 0

    /** Whether the last point made the players change ends. */
    private var changeEnds = false

    init {
        playing = firstOfSet()
    }

    /** Award a point to [team]. Returns false, changing nothing, when the match is over. */
    fun apply(team: Team): Boolean {
        when (val u = playing) {
            Playing.Over -> return false
            is Playing.InGame -> {
                changeEnds = false
                u.game.point(team)?.let { gameWon(it, null) }
            }
            is Playing.InTieBreak -> {
                val won = u.tb.point(team)
                val points = listOf(u.tb.a, u.tb.b)
                changeEnds = endsChangeInTiebreak(u.tb.pointsPlayed)
                if (won != null) {
                    if (u.superTiebreak) {
                        // A super tie-break is its set's only game.
                        gameFinished(1)
                        val (a, b) = if (won == Team.A) 1 to 0 else 0 to 1
                        setWon(won, SetScore(a, b, points))
                    } else {
                        gameWon(won, points)
                    }
                }
            }
        }
        return true
    }

    /** [w] won a game (or a regular tie-break, which counts as the set's 13th game). */
    private fun gameWon(w: Team, tiebreak: List<Int>?) {
        games[w.slot]++
        val (a, b) = games
        gameFinished(a + b)
        val setWinner = setWon(a, b)
        if (setWinner != null) {
            setWon(setWinner, SetScore(a, b, tiebreak))
        } else if (a == SET_GAMES && b == SET_GAMES) {
            playing = Playing.InTieBreak(TieBreak(TIEBREAK_TO), superTiebreak = false)
        } else {
            playing = Playing.InGame(Game(rules.deuce))
        }
    }

    /**
     * A game (or tie-break) ended with [gamesInSet] games now played in its set: pass service to the
     * next player in rotation and apply the change-of-ends rule. After a tie-break that player is the
     * one after the tie-break's first server.
     */
    private fun gameFinished(gamesInSet: Int) {
        rotation = (rotation + 1) % 4
        changeEnds = changeEnds || endsChangeAfterGame(gamesInSet)
    }

    private fun setWon(w: Team, set: SetScore) {
        games[0] = set.a
        games[1] = set.b
        finished += set
        setsWon[w.slot]++
        if (setsWon[w.slot] >= rules.sets.setsToWin) {
            winner = w
            playing = Playing.Over
        } else {
            games.fill(0)
            playing = firstOfSet()
        }
    }

    /** A game, or a super tie-break when this is the deciding set and the rules call for one. */
    private fun firstOfSet(): Playing {
        val oneShort = rules.sets.setsToWin - 1
        val deciding = setsWon[0] == oneShort && setsWon[1] == oneShort
        return if (deciding && rules.deciding == DecidingSet.SUPER_TIE_BREAK) {
            Playing.InTieBreak(TieBreak(SUPER_TIEBREAK_TO), superTiebreak = true)
        } else {
            Playing.InGame(Game(rules.deuce))
        }
    }

    fun score(): EngineScore {
        val (a, b) = games
        val sets = finished.toMutableList()
        val (server, side) = when (val u = playing) {
            Playing.Over -> inRotation(start, rotation) to ServeSide.RIGHT
            is Playing.InGame ->
                inRotation(start, rotation) to serveSide(u.game.pointsPlayed, u.game.score().decidingPoint)
            is Playing.InTieBreak ->
                tiebreakServer(start, rotation, u.tb.pointsPlayed) to serveSide(u.tb.pointsPlayed, false)
        }
        val current = when (val u = playing) {
            Playing.Over -> Current.None
            is Playing.InGame -> {
                sets += SetScore(a, b)
                Current.Game(u.game.score())
            }
            is Playing.InTieBreak -> {
                sets += SetScore(a, b, listOf(u.tb.a, u.tb.b))
                Current.TieBreak(TieBreakScore(u.tb.a, u.tb.b, u.superTiebreak))
            }
        }
        val status = winner?.let { Status.Finished(it) } ?: Status.InProgress
        return EngineScore(
            sets = sets,
            setsWon = setsWon.toList(),
            games = listOf(a, b),
            regulationGames = listOf(a, b),
            current = current,
            server = server,
            side = side,
            // No change of ends after match point: the match is over.
            changeEnds = changeEnds && status == Status.InProgress,
            overtime = false,
            timeUp = false,
            status = status,
        )
    }

    companion object {
        /** A new match, or null when the rules or the start are invalid. */
        fun create(rules: SetRules, start: Start): SetsMatch? =
            if (rules.valid && start.valid) SetsMatch(rules, start) else null
    }
}

/** The score after [points], or null when the rules or start are invalid or any point is refused. */
fun replay(rules: SetRules, start: Start, points: List<Team>): EngineScore? {
    val match = SetsMatch.create(rules, start) ?: return null
    return if (points.all(match::apply)) match.score() else null
}

private val Team.slot: Int get() = when (this) {
    Team.A -> 0
    Team.B -> 1
}
