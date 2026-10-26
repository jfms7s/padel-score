package io.github.jfms7s.padelscore

import android.net.Uri
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.res.stringResource
import androidx.lifecycle.viewmodel.compose.viewModel
import androidx.navigation.NavHostController
import androidx.wear.compose.navigation.SwipeDismissableNavHost
import androidx.wear.compose.navigation.composable
import androidx.wear.compose.navigation.rememberSwipeDismissableNavController
import io.github.jfms7s.padelscore.auth.SignInModel
import io.github.jfms7s.padelscore.auth.SignInScreen
import io.github.jfms7s.padelscore.matches.ListState
import io.github.jfms7s.padelscore.matches.MatchListModel
import io.github.jfms7s.padelscore.matches.MatchListScreen
import io.github.jfms7s.padelscore.score.ScoreModel
import io.github.jfms7s.padelscore.score.ScoreScreen
import io.github.jfms7s.padelscore.settings.SettingsModel
import io.github.jfms7s.padelscore.settings.SettingsScreen
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import java.io.IOException

private const val SIGN_IN = "signin"
private const val MATCHES = "matches"
private const val SCORE = "score/{id}"

private const val SETTINGS = "settings"

/** [onSettingsApplied] runs once new server settings are saved; the activity then restarts on them. */
@Composable
fun WatchApp(container: AppContainer, onSettingsApplied: () -> Unit) =
    WatchApp(container, rememberSwipeDismissableNavController(), onSettingsApplied)

/** [nav] is a seam for tests, which check the back stack after a sign-out. */
@Composable
internal fun WatchApp(container: AppContainer, nav: NavHostController, onSettingsApplied: () -> Unit) {
    // signedIn only knows a token blob for this server exists; a blob that no longer decrypts is cleared on the
    // first read, the first call then answers Unauthorized, and the sign-out below takes over.
    val start by produceState<String?>(null, container) {
        value = try {
            if (container.tokens.signedIn.first()) MATCHES else SIGN_IN
        } catch (_: IOException) {
            SIGN_IN
        }
    }
    Box(Modifier.fillMaxSize().background(Color.Black)) {
        start?.let { WatchNavHost(container, nav, it, onSettingsApplied) }
    }
}

@Composable
private fun WatchNavHost(
    container: AppContainer,
    nav: NavHostController,
    start: String,
    onSettingsApplied: () -> Unit,
) {
    val signOut = rememberSignOut(container, nav)
    val openSettings = { nav.navigate(SETTINGS) { launchSingleTop = true } }
    SwipeDismissableNavHost(navController = nav, startDestination = start) {
        composable(SIGN_IN) {
            val model = viewModel {
                SignInModel(container.api, container.tokens, container.serverUrl, container.deviceName)
            }
            SignInScreen(
                model,
                onSignedIn = { nav.navigate(MATCHES) { popUpTo(SIGN_IN) { inclusive = true } } },
                onSettings = openSettings,
            )
        }
        composable(MATCHES) {
            val title = stringResource(R.string.quick_match)
            val model = viewModel { MatchListModel(container.api, container.listStream(), title) }
            val state by model.state.collectAsState()
            LaunchedEffect(state) {
                if (state == ListState.Unauthorized) signOut()
            }
            MatchListScreen(
                model,
                onPick = { id -> nav.navigate("score/${Uri.encode(id)}") },
                onSettings = openSettings,
                onSignOut = signOut,
            )
        }
        composable(SCORE) { entry ->
            val id = entry.arguments?.getString("id") ?: return@composable
            val model = viewModel {
                ScoreModel(id, container.api, container.matchStream(id), container.haptics)
            }
            val unauthorized by model.unauthorized.collectAsState()
            LaunchedEffect(unauthorized) {
                if (unauthorized) signOut()
            }
            ScoreScreen(model, onDone = { nav.popBackStack(MATCHES, inclusive = false) })
        }
        composable(SETTINGS) {
            val model = viewModel { SettingsModel(container.settingsStore, container.settings, container.tokens) }
            SettingsScreen(model, onApplied = onSettingsApplied)
        }
    }
}

/**
 * Forgets the token and shows sign-in with nothing to go back to. Several models can report a 401
 * for the same revoke (list GET, list stream, match stream, a command), so repeats are dropped.
 */
@Composable
private fun rememberSignOut(container: AppContainer, nav: NavHostController): () -> Unit {
    val scope = rememberCoroutineScope()
    val busy = remember { mutableStateOf(false) }
    return remember(container, nav) {
        {
            if (!busy.value && nav.currentDestination?.route != SIGN_IN) {
                busy.value = true
                scope.launch {
                    try {
                        container.tokens.clear()
                    } catch (_: IOException) {
                        // Still leave: the next sign-in overwrites the token, and staying here would loop on 401s.
                    }
                    nav.navigate(SIGN_IN) { popUpTo(nav.graph.id) { inclusive = true } }
                    busy.value = false
                }
            }
        }
    }
}
