package io.github.jfms7s.padelscore

import android.content.Context
import kotlinx.serialization.json.Json
import okhttp3.HttpUrl
import okhttp3.OkHttpClient
import java.time.Duration

/** Manual dependency injection: one instance per process, built by [PadelApp]. */
class AppContainer(context: Context, val serverUrl: HttpUrl) {
    val appContext: Context = context.applicationContext

    val json = Json { ignoreUnknownKeys = true }

    val http: OkHttpClient = OkHttpClient.Builder()
        .connectTimeout(Duration.ofSeconds(10))
        .callTimeout(Duration.ofSeconds(15))
        .build()

    // SSE stays open indefinitely; the stream reader owns silence detection, not OkHttp.
    val streamHttp: OkHttpClient = http.newBuilder()
        .callTimeout(Duration.ZERO)
        .readTimeout(Duration.ZERO)
        .build()
}
