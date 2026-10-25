package io.github.jfms7s.padelscore.net

import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonElement

// A subset of the server's JSON: only what the watch reads. Unknown fields are dropped on decode.

val WireJson: Json = Json {
    ignoreUnknownKeys = true
    classDiscriminator = "kind"
    explicitNulls = false
    encodeDefaults = true
}

@Serializable
enum class Team {
    @SerialName("a") A,
    @SerialName("b") B,
}

@Serializable
enum class PointCall {
    @SerialName("love") LOVE,
    @SerialName("fifteen") FIFTEEN,
    @SerialName("thirty") THIRTY,
    @SerialName("forty") FORTY,
    @SerialName("advantage") ADVANTAGE,
}

@Serializable
data class Player(val team: Team, val index: Int)

@Serializable
data class Teams(val a: List<String>, val b: List<String>)

@Serializable
data class GameScore(
    val a: PointCall,
    val b: PointCall,
    val deuces: Int,
    @SerialName("deciding_point") val decidingPoint: Boolean,
)

@Serializable
data class TieBreakScore(
    val a: Int,
    val b: Int,
    @SerialName("super_tiebreak") val superTiebreak: Boolean,
)

@Serializable(with = CurrentSerializer::class)
sealed interface Current {
    data class Game(val score: GameScore) : Current
    data class TieBreak(val score: TieBreakScore) : Current
    data object None : Current
}

@Serializable(with = StatusSerializer::class)
sealed interface Status {
    data object InProgress : Status
    data class Finished(val winner: Team?) : Status
}

@Serializable
data class Score(
    @SerialName("sets_won") val setsWon: List<Int>,
    val games: List<Int>,
    val current: Current,
    val server: Player,
    @SerialName("change_ends") val changeEnds: Boolean,
    val status: Status,
)

// The event payload is opaque to the watch; only its seq matters (for undo targets).
@Serializable
data class ViewEvent(val seq: Long, val event: JsonElement)

@Serializable
data class MatchView(
    val id: String,
    val teams: Teams,
    val revision: Long,
    @SerialName("head_seq") val headSeq: Long,
    val events: List<ViewEvent>,
    val score: Score,
)

val MatchView.lastEffectiveSeq: Long? get() = events.lastOrNull()?.seq

@Serializable
sealed interface Command {
    @Serializable
    @SerialName("point")
    data class Point(val team: Team) : Command

    @Serializable
    @SerialName("undo")
    data class Undo(@SerialName("target_seq") val targetSeq: Long) : Command
}

@Serializable
data class CommandRequest(
    @SerialName("client_id") val clientId: String,
    val batch: Boolean = false,
    @SerialName("base_revision") val baseRevision: Long,
    val commands: List<Command>,
)

@Serializable
data class DeviceStart(
    @SerialName("device_code") val deviceCode: String,
    @SerialName("user_code") val userCode: String,
    @SerialName("verification_uri") val verificationUri: String,
    @SerialName("expires_in") val expiresIn: Int,
    val interval: Int,
)

@Serializable
data class DeviceToken(@SerialName("access_token") val accessToken: String)

@Serializable
data class ErrorBody(
    val error: String,
    val view: MatchView? = null,
    val kind: JsonElement? = null,
    @SerialName("retry_after_secs") val retryAfterSecs: Long? = null,
)

@Serializable
data class MyMatch(val id: String, val label: String? = null, val view: MatchView? = null)
