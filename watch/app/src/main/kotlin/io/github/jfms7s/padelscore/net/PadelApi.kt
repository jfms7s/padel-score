package io.github.jfms7s.padelscore.net

import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import kotlinx.serialization.KSerializer
import kotlinx.serialization.builtins.ListSerializer
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put
import okhttp3.HttpUrl
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.RequestBody.Companion.toRequestBody
import okhttp3.Response
import java.io.IOException

sealed interface ApiResult<out T> {
    data class Ok<T>(val value: T) : ApiResult<T>
    data object Unauthorized : ApiResult<Nothing>
    data class TooMany(val retryAfterSeconds: Long?) : ApiResult<Nothing>
    data class Failed(val reason: String) : ApiResult<Nothing>
}

sealed interface TokenPoll {
    data class Granted(val token: String) : TokenPoll
    data object Pending : TokenPoll
    data object SlowDown : TokenPoll
    data object Expired : TokenPoll
    data object Denied : TokenPoll
    data class TooMany(val retryAfterSeconds: Long?) : TokenPoll
    data class Failed(val reason: String) : TokenPoll
}

sealed interface SendResult {
    data class Applied(val view: MatchView) : SendResult
    data class Conflict(val view: MatchView) : SendResult
    data class Rejected(val kind: JsonElement) : SendResult
    data object Unauthorized : SendResult
    data object NotFound : SendResult
    data class NotSent(val reason: String) : SendResult
}

interface PadelApi {
    suspend fun startDevice(deviceName: String): ApiResult<DeviceStart>
    suspend fun pollToken(deviceCode: String): TokenPoll
    suspend fun myMatches(): ApiResult<List<MyMatch>>
    suspend fun send(matchId: String, request: CommandRequest): SendResult
    suspend fun listStreamRequest(): Request
    suspend fun matchStreamRequest(matchId: String): Request
}

/** Never logs tokens or bodies; callers get a typed outcome, not an exception. */
class HttpPadelApi(
    private val baseUrl: HttpUrl,
    private val http: OkHttpClient,
    private val json: Json,
    private val token: suspend () -> String?,
) : PadelApi {

    override suspend fun startDevice(deviceName: String): ApiResult<DeviceStart> {
        val request = Request.Builder()
            .url(url("api", "device", "start"))
            .post(jsonBody(buildJsonObject { put("device_name", deviceName.take(DEVICE_NAME_MAX)) }))
            .build()
        return call(request) { response, body ->
            when {
                response.code == 200 -> decode(DeviceStart.serializer(), body)
                    ?.let { ApiResult.Ok(it) } ?: ApiResult.Failed("undecodable body")
                response.code == 429 -> ApiResult.TooMany(retryAfter(response, body))
                else -> ApiResult.Failed("HTTP ${response.code}")
            }
        } ?: ApiResult.Failed("network error")
    }

    override suspend fun pollToken(deviceCode: String): TokenPoll {
        val request = Request.Builder()
            .url(url("api", "device", "token"))
            .post(jsonBody(buildJsonObject { put("device_code", deviceCode) }))
            .build()
        return call(request) { response, body ->
            val code = response.code
            when {
                code == 200 -> decode(DeviceToken.serializer(), body)
                    ?.let { TokenPoll.Granted(it.accessToken) } ?: TokenPoll.Failed("undecodable body")
                code == 429 -> TokenPoll.TooMany(retryAfter(response, body))
                code == 400 -> when (decode(ErrorBody.serializer(), body)?.error) {
                    "authorization_pending" -> TokenPoll.Pending
                    "slow_down" -> TokenPoll.SlowDown
                    "access_denied" -> TokenPoll.Denied
                    else -> TokenPoll.Expired
                }
                code in 400..499 -> TokenPoll.Expired
                else -> TokenPoll.Failed("HTTP $code")
            }
        } ?: TokenPoll.Failed("network error")
    }

    override suspend fun myMatches(): ApiResult<List<MyMatch>> {
        val bearer = token() ?: return ApiResult.Unauthorized
        val request = Request.Builder()
            .url(url("api", "me", "matches"))
            .header("Authorization", "Bearer $bearer")
            .build()
        return call(request) { response, body ->
            when (response.code) {
                200 -> decode(ListSerializer(MyMatch.serializer()), body)
                    ?.let { ApiResult.Ok(it) } ?: ApiResult.Failed("undecodable body")
                401 -> ApiResult.Unauthorized
                429 -> ApiResult.TooMany(retryAfter(response, body))
                else -> ApiResult.Failed("HTTP ${response.code}")
            }
        } ?: ApiResult.Failed("network error")
    }

    override suspend fun send(matchId: String, request: CommandRequest): SendResult {
        val bearer = token() ?: return SendResult.Unauthorized
        val http = Request.Builder()
            .url(url("api", "me", "matches", matchId, "commands"))
            .header("Authorization", "Bearer $bearer")
            .post(jsonBody(request))
            .build()
        return call(http) { response, body ->
            when (val code = response.code) {
                200 -> decode(MatchView.serializer(), body)?.let { SendResult.Applied(it) }
                    ?: SendResult.NotSent("undecodable body")
                409 -> decode(ErrorBody.serializer(), body)?.view?.let { SendResult.Conflict(it) }
                    ?: SendResult.NotSent("undecodable conflict")
                422 -> SendResult.Rejected(decode(ErrorBody.serializer(), body)?.kind ?: JsonNull)
                401 -> SendResult.Unauthorized
                404, 403 -> SendResult.NotFound
                // Includes 400 invalid: the server answered, but it is a client bug, not a finished match.
                else -> SendResult.NotSent("HTTP $code")
            }
        } ?: SendResult.NotSent("network error")
    }

    // Suspend (unlike the brief's plain fun) because the bearer token is read through the suspend lookup.
    override suspend fun listStreamRequest(): Request = streamRequest(url("api", "me", "matches", "stream"))

    override suspend fun matchStreamRequest(matchId: String): Request =
        streamRequest(url("api", "me", "matches", matchId, "stream"))

    private suspend fun streamRequest(url: HttpUrl): Request =
        Request.Builder().url(url).header("Accept", "text/event-stream").apply {
            token()?.let { header("Authorization", "Bearer $it") }
        }.build()

    private fun jsonBody(body: JsonObject) = body.toString().toRequestBody(JSON_MEDIA)

    private fun jsonBody(body: CommandRequest) =
        json.encodeToString(CommandRequest.serializer(), body).toRequestBody(JSON_MEDIA)

    private fun url(vararg segments: String): HttpUrl =
        baseUrl.newBuilder().apply { segments.forEach(::addPathSegment) }.build()

    /** Runs the call on IO; null means the request never produced a response (IO error or timeout). */
    private suspend fun <T> call(request: Request, map: (Response, String) -> T): T? =
        withContext(Dispatchers.IO) {
            try {
                http.newCall(request).execute().use { response -> map(response, response.body.string()) }
            } catch (_: IOException) {
                null
            }
        }

    private fun <T> decode(serializer: KSerializer<T>, body: String): T? =
        try {
            json.decodeFromString(serializer, body)
        } catch (_: IllegalArgumentException) {
            null // SerializationException is an IllegalArgumentException
        }

    private fun retryAfter(response: Response, body: String): Long? =
        response.header("Retry-After")?.trim()?.toLongOrNull()
            ?: decode(ErrorBody.serializer(), body)?.retryAfterSecs

    private companion object {
        val JSON_MEDIA = "application/json".toMediaType()
        const val DEVICE_NAME_MAX = 40
    }
}
