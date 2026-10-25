package io.github.jfms7s.padelscore

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.runtime.Composable
import androidx.compose.runtime.produceState
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.wear.compose.material3.MaterialTheme
import androidx.wear.compose.material3.Text
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import kotlinx.serialization.Serializable
import okhttp3.Request
import java.io.IOException

// Throwaway end-to-end check that the watch reaches the server; replaced by WatchApp.

@Serializable
private data class Health(val version: String)

private sealed interface HealthState {
    data object Checking : HealthState
    data class Up(val version: String) : HealthState
    data object Unreachable : HealthState
}

@Composable
fun HealthTracer(container: AppContainer) {
    val state = produceState<HealthState>(HealthState.Checking, container) {
        value = fetchHealth(container)
    }.value
    val text = when (state) {
        HealthState.Checking -> stringResource(R.string.tracer_checking)
        is HealthState.Up -> stringResource(R.string.tracer_server_version, state.version)
        HealthState.Unreachable -> stringResource(R.string.tracer_unreachable)
    }
    MaterialTheme {
        Box(Modifier.fillMaxSize().padding(16.dp), contentAlignment = Alignment.Center) {
            Text(text, textAlign = TextAlign.Center)
        }
    }
}

private suspend fun fetchHealth(container: AppContainer): HealthState = withContext(Dispatchers.IO) {
    val request = Request.Builder().url(container.serverUrl.resolve("api/health")!!).build()
    try {
        container.http.newCall(request).execute().use { response ->
            if (!response.isSuccessful) return@use HealthState.Unreachable
            HealthState.Up(container.json.decodeFromString<Health>(response.body.string()).version)
        }
    } catch (_: IOException) {
        HealthState.Unreachable
    } catch (_: kotlinx.serialization.SerializationException) {
        HealthState.Unreachable
    }
}
