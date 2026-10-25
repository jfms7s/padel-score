package io.github.jfms7s.padelscore

import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.v2.createComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import mockwebserver3.MockResponse
import mockwebserver3.MockWebServer
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class HealthTracerTest {
    @get:Rule val compose = createComposeRule()

    private val server = MockWebServer()

    @Before fun startServer() = server.start()

    @After fun stopServer() = server.close()

    private fun showTracer() {
        val container = AppContainer(ApplicationProvider.getApplicationContext(), server.url("/"))
        compose.setContent { HealthTracer(container) }
    }

    private fun waitForText(text: String) {
        compose.waitUntil(timeoutMillis = 5_000) {
            compose.onAllNodes(hasText(text)).fetchSemanticsNodes().isNotEmpty()
        }
        compose.onNodeWithText(text).assertExists()
    }

    @Test
    fun shows_the_server_version() {
        server.enqueue(
            MockResponse.Builder()
                .addHeader("Content-Type", "application/json")
                .body("""{"status":"ok","version":"v0.2.0","commit":"abc"}""")
                .build(),
        )

        showTracer()

        waitForText("Server v0.2.0")
        assertEquals("/api/health", server.takeRequest().url.encodedPath)
    }

    @Test
    fun says_when_the_server_is_unavailable() {
        server.enqueue(MockResponse.Builder().code(503).build())

        showTracer()

        waitForText("Can't reach the server")
    }
}

