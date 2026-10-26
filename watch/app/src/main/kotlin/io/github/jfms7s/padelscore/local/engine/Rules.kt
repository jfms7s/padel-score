package io.github.jfms7s.padelscore.local.engine

import io.github.jfms7s.padelscore.net.Current
import io.github.jfms7s.padelscore.net.Player
import io.github.jfms7s.padelscore.net.Score
import io.github.jfms7s.padelscore.net.Status
import io.github.jfms7s.padelscore.net.Team
import io.github.jfms7s.padelscore.net.WireJson
import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable

// A port of the sets match in crates/scoring, so the watch can score a match offline. The serial
// names are serde's, and EngineVectorsTest checks every step against the Rust engine's own output.

@Serializable
enum class DeuceRule {
    @SerialName("advantage") ADVANTAGE,
    @SerialName("golden_point") GOLDEN_POINT,
    @SerialName("star_point") STAR_POINT,
}

/** With [DeuceRule.STAR_POINT], the deciding point is played at the third deuce. */
internal const val STAR_POINT_DEUCE = 3

@Serializable
enum class SetsFormat {
    @SerialName("one") ONE,
    @SerialName("best_of_3") BEST_OF_3,
    ;

    val setsToWin: Int get() = when (this) {
        ONE -> 1
        BEST_OF_3 -> 2
    }
}

@Serializable
enum class DecidingSet {
    @SerialName("full_set") FULL_SET,
    @SerialName("super_tie_break") SUPER_TIE_BREAK,
}

@Serializable
data class SetRules(val deuce: DeuceRule, val sets: SetsFormat, val deciding: DecidingSet) {
    /** A one-set match has no deciding set to replace with a super tie-break. */
    val valid: Boolean get() = !(sets == SetsFormat.ONE && deciding == DecidingSet.SUPER_TIE_BREAK)
}

@Serializable
data class Start(
    @SerialName("first_server") val firstServer: Player,
    /** The index (0 or 1) of the player who serves first for the team that receives first. */
    @SerialName("other_first_server") val otherFirstServer: Int,
) {
    val valid: Boolean get() = firstServer.index in 0..1 && otherFirstServer in 0..1
}

/** Which side of the court the next serve goes from. */
@Serializable
enum class ServeSide {
    @SerialName("right") RIGHT,
    @SerialName("left") LEFT,
    @SerialName("receiver_chooses") RECEIVER_CHOOSES,
}

/** A set's score. For a super tie-break set, a/b are 1-0 or 0-1 and [tiebreak] holds the points. */
@Serializable
data class SetScore(val a: Int, val b: Int, val tiebreak: List<Int>? = null)

/** The engine's `Score` for a sets match: `overtime` and `timeUp` are always false. */
@Serializable
data class EngineScore(
    /** The finished sets, then the one in progress. */
    val sets: List<SetScore>,
    @SerialName("sets_won") val setsWon: List<Int>,
    /** Games in the current set; once the match is over, the final set's. */
    val games: List<Int>,
    @SerialName("regulation_games") val regulationGames: List<Int>,
    val current: Current,
    /** Who serves the next point. Meaningless once the match is over. */
    val server: Player,
    /** Where the next serve goes from. Meaningless once the match is over. */
    val side: ServeSide,
    /** True only right after the point that caused it, and never once the match is over. */
    @SerialName("change_ends") val changeEnds: Boolean,
    val overtime: Boolean,
    @SerialName("time_up") val timeUp: Boolean,
    val status: Status,
)

/** The engine score in the shape the score screen reads from the server. */
fun EngineScore.toWire(): Score = Score(
    setsWon = setsWon,
    games = games,
    current = current,
    server = server,
    changeEnds = changeEnds,
    status = status,
    sets = sets.map { WireJson.encodeToJsonElement(SetScore.serializer(), it) },
)

internal fun Team.other(): Team = when (this) {
    Team.A -> Team.B
    Team.B -> Team.A
}
