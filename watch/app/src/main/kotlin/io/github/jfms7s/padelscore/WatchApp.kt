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
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
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
import io.github.jfms7s.padelscore.local.LocalDecidingScreen
import io.github.jfms7s.padelscore.local.LocalDeps
import io.github.jfms7s.padelscore.local.LocalDeuceScreen
import io.github.jfms7s.padelscore.local.LocalEntryButtons
import io.github.jfms7s.padelscore.local.LocalMatch
import io.github.jfms7s.padelscore.local.LocalMatchModel
import io.github.jfms7s.padelscore.local.LocalOtherServerScreen
import io.github.jfms7s.padelscore.local.LocalScoreScreen
import io.github.jfms7s.padelscore.local.LocalServeScreen
import io.github.jfms7s.padelscore.local.LocalSetsScreen
import io.github.jfms7s.padelscore.local.engine.SetsFormat
import io.github.jfms7s.padelscore.matches.ListState
import io.github.jfms7s.padelscore.matches.MatchListModel
import io.github.jfms7s.padelscore.matches.MatchListScreen
import io.github.jfms7s.padelscore.score.ScoreModel
import io.github.jfms7s.padelscore.score.ScoreScreen
import io.github.jfms7s.padelscore.settings.SettingsModel
import io.github.jfms7s.padelscore.settings.FirstRunSettings
import io.github.jfms7s.padelscore.settings.SettingsScreen
import io.github.jfms7s.padelscore.settings.SettingsStore
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import java.io.IOException

private const val FIRST_RUN = "first-run"
private const val SIGN_IN = "signin"
private const val MATCHES = "matches"
private const val SCORE = "score/{id}"

private const val SETTINGS = "settings"

private const val LOCAL_DEUCE = "local/deuce"
private const val LOCAL_SETS = "local/sets"
private const val LOCAL_DECIDING = "local/deciding"
private const val LOCAL_SERVE = "local/serve"
private const val LOCAL_OTHER = "local/other"
private const val LOCAL_SCORE = "local/score"

/**
 * [container] is null until a server is set; [settingsStore] is where the first run sets one. [onSettingsApplied]
 * runs once new server settings are saved; the activity then restarts on them.
 */
@Composable
fun WatchApp(
    container: AppContainer?,
    settingsStore: SettingsStore,
    local: LocalDeps,
    onSettingsApplied: () -> Unit,
) = WatchApp(container, settingsStore, local, rememberSwipeDismissableNavController(), onSettingsApplied)

/** [nav] is a seam for tests, which check the back stack after a sign-out. */
@Composable
internal fun WatchApp(
    container: AppContainer?,
    settingsStore: SettingsStore,
    local: LocalDeps,
    nav: NavHostController,
    onSettingsApplied: () -> Unit,
) {
    // Activity-scoped, so every route shares the one local match.
    val localModel = viewModel { LocalMatchModel(local.store, local.haptics) }
    // signedIn only knows a token blob for this server exists; a blob that no longer decrypts is cleared on the
    // first read, the first call then answers Unauthorized, and the sign-out below takes over.
    val start by produceState<String?>(null, container) {
        val home = when {
            container == null -> FIRST_RUN
            else -> try {
                if (container.tokens.signedIn.first()) MATCHES else SIGN_IN
            } catch (_: IOException) {
                SIGN_IN
            }
        }
        // Wait for the local match too, so a stored one can open straight away.
        localModel.ui.first { it.loaded }
        value = home
    }
    Box(Modifier.fillMaxSize().background(Color.Black)) {
        start?.let { WatchNavHost(container, settingsStore, localModel, nav, it, onSettingsApplied) }
    }
}

@Composable
private fun WatchNavHost(
    container: AppContainer?,
    settingsStore: SettingsStore,
    localModel: LocalMatchModel,
    nav: NavHostController,
    start: String,
    onSettingsApplied: () -> Unit,
) {
    val signOut = if (container != null) rememberSignOut(container, nav) else ({})
    val openSettings = { nav.navigate(SETTINGS) { launchSingleTop = true } }
    val localEntry: @Composable () -> Unit = { LocalEntry(localModel, nav) }
    // Opening the app with a local match in progress goes to its score, with the usual first screen under it.
    var resumeChecked by rememberSaveable { mutableStateOf(false) }
    LaunchedEffect(Unit) {
        if (!resumeChecked) {
            resumeChecked = true
            if (localModel.ui.value.match != null) nav.navigate(LOCAL_SCORE)
        }
    }
    SwipeDismissableNavHost(navController = nav, startDestination = start) {
        composable(FIRST_RUN) {
            FirstRunSettings(settingsStore, onApplied = onSettingsApplied, localEntry = localEntry)
        }
        if (container != null) {
            composable(SIGN_IN) {
                val model = viewModel {
                    SignInModel(container.api, container.tokens, container.serverUrl, container.deviceName)
                }
                SignInScreen(
                    model,
                    onSignedIn = { nav.navigate(MATCHES) { popUpTo(SIGN_IN) { inclusive = true } } },
                    onSettings = openSettings,
                    localEntry = localEntry,
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
                    localEntry = localEntry,
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
        composable(LOCAL_DEUCE) {
            LocalDeuceScreen(localModel, onNext = { nav.navigate(LOCAL_SETS) { launchSingleTop = true } })
        }
        composable(LOCAL_SETS) {
            LocalSetsScreen(localModel, onNext = { sets ->
                val next = if (sets == SetsFormat.BEST_OF_3) LOCAL_DECIDING else LOCAL_SERVE
                nav.navigate(next) { launchSingleTop = true }
            })
        }
        composable(LOCAL_DECIDING) {
            LocalDecidingScreen(localModel, onNext = { nav.navigate(LOCAL_SERVE) { launchSingleTop = true } })
        }
        composable(LOCAL_SERVE) {
            LocalServeScreen(localModel, onNext = { nav.navigate(LOCAL_OTHER) { launchSingleTop = true } })
        }
        composable(LOCAL_OTHER) {
            LocalOtherRoute(localModel, nav)
        }
        composable(LOCAL_SCORE) {
            val ui by localModel.ui.collectAsState()
            // Gone from under the screen (a discard elsewhere): nothing left to show here.
            LaunchedEffect(ui.loaded, ui.match == null) {
                if (ui.loaded && ui.match == null && nav.currentDestination?.route == LOCAL_SCORE) nav.popBackStack()
            }
            // Only the local score is popped, and only while it is the one showing: a second tap during the exit
            // animation must not take the home screen with it.
            LocalScoreScreen(localModel, onBack = { nav.popBackStack(LOCAL_SCORE, inclusive = true) })
        }
    }
}

/** The local match buttons every first screen shows. */
@Composable
private fun LocalEntry(model: LocalMatchModel, nav: NavHostController) {
    val ui by model.ui.collectAsState()
    LocalEntryButtons(
        ui,
        onNew = { nav.navigate(LOCAL_DEUCE) { launchSingleTop = true } },
        onResume = { nav.navigate(LOCAL_SCORE) { launchSingleTop = true } },
        onDiscard = model::discard,
    )
}

/**
 * The last setup pick starts the match, and the score opens once it is saved, with the setup screens gone so back
 * leads to the first screen. The save is a disk write; opening the score before it lands would find no match there.
 * A failed save stays here (the refused haptic says so), and the pick can be tapped again.
 */
@Composable
private fun LocalOtherRoute(model: LocalMatchModel, nav: NavHostController) {
    var starting by remember { mutableStateOf<Starting?>(null) }
    LocalOtherServerScreen(model, onNext = { setup ->
        if (starting == null) {
            starting = Starting(LocalMatch(setup, emptyList()), model.ui.value.feedbackId)
            model.start(setup)
        }
    })
    LaunchedEffect(starting) {
        val start = starting ?: return@LaunchedEffect
        // The only feedback a start gives is Not saved.
        val ui = model.ui.first { it.match == start.match || it.feedbackId != start.feedbackId }
        if (ui.match != start.match) {
            starting = null
        } else if (nav.currentDestination?.route == LOCAL_OTHER) {
            nav.navigate(LOCAL_SCORE) { popUpTo(LOCAL_DEUCE) { inclusive = true } }
        }
    }
}

/** A start in flight: the new [match], and the feedback id before it. */
private data class Starting(val match: LocalMatch, val feedbackId: Long)

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
