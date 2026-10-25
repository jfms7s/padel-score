package io.github.jfms7s.padelscore

import android.content.Context
import io.github.jfms7s.padelscore.net.HttpPadelApi
import io.github.jfms7s.padelscore.net.PadelApi
import io.github.jfms7s.padelscore.net.WireJson
import kotlinx.serialization.json.Json
import okhttp3.HttpUrl
import okhttp3.OkHttpClient
import java.time.Duration

/** Manual dependency injection: one instance per process, built by [PadelApp]. */
class AppContainer(context: Context, val serverUrl: HttpUrl) {
    val appContext: Context = context.applicationContext

    val json = Json { ignoreUnknownKeys = true }

    val http: OkHttpClient = OkHttpClient.Builder()
        .connectTimeout(Duration.ofSeconds(5))
        .callTimeout(Duration.ofSeconds(8))
        .retryOnConnectionFailure(false)
        .build()

    // SSE stays open indefinitely, but the server keep-alives every 10 s, so 20 s of read silence means a dead link.
    val streamHttp: OkHttpClient = http.newBuilder()
        .callTimeout(Duration.ZERO)
        .readTimeout(Duration.ofSeconds(20))
        .build()

    // Token lookup is a placeholder until TokenStore (Task 6) and the wiring (Task 10).
    val api: PadelApi = HttpPadelApi(serverUrl, http, WireJson, token = { null })
}
