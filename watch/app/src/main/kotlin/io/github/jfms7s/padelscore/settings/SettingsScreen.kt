package io.github.jfms7s.padelscore.settings

import android.app.Activity
import android.app.RemoteInput
import android.content.ActivityNotFoundException
import android.view.inputmethod.EditorInfo
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.wear.compose.foundation.lazy.TransformingLazyColumn
import androidx.wear.compose.foundation.lazy.rememberTransformingLazyColumnState
import androidx.wear.compose.material3.AlertDialog
import androidx.wear.compose.material3.Button
import androidx.wear.compose.material3.ButtonDefaults
import androidx.wear.compose.material3.MaterialTheme
import androidx.wear.compose.material3.ScreenScaffold
import androidx.wear.compose.material3.Text
import androidx.wear.input.RemoteInputIntentHelper
import androidx.wear.input.wearableExtender
import io.github.jfms7s.padelscore.R

@Composable
fun SettingsScreen(model: SettingsModel, onApplied: () -> Unit) {
    val state by model.state.collectAsState()
    LaunchedEffect(state.applied) {
        if (state.applied) onApplied()
    }
    val editServer = rememberTextInput(stringResource(R.string.settings_server_input), model::submitServer)
    val editAddresses = rememberTextInput(stringResource(R.string.settings_addresses_input), model::submitAddresses)
    SettingsContent(state, onEditServer = editServer, onEditAddresses = editAddresses, onReset = model::reset)
}

private const val INPUT_KEY = "text"

/**
 * Opens the system keyboard / voice input and hands back what was entered. The RemoteInput API has
 * no way to prefill the field, so the screen shows the current value next to the row instead.
 */
@Composable
private fun rememberTextInput(label: String, onText: (String) -> Unit): () -> Unit {
    val currentOnText by rememberUpdatedState(onText)
    val launcher = rememberLauncherForActivityResult(ActivityResultContracts.StartActivityForResult()) { result ->
        val data = result.data
        if (result.resultCode == Activity.RESULT_OK && data != null) {
            RemoteInput.getResultsFromIntent(data)?.getCharSequence(INPUT_KEY)?.let { currentOnText(it.toString()) }
        }
    }
    return remember(launcher, label) {
        {
            val input = RemoteInput.Builder(INPUT_KEY)
                .setLabel(label)
                .wearableExtender {
                    setEmojisAllowed(false)
                    setInputActionType(EditorInfo.IME_ACTION_DONE)
                }
                .build()
            val intent = RemoteInputIntentHelper.createActionRemoteInputIntent()
            RemoteInputIntentHelper.putRemoteInputsExtra(intent, listOf(input))
            RemoteInputIntentHelper.putTitleExtra(intent, label)
            try {
                launcher.launch(intent)
            } catch (_: ActivityNotFoundException) {
                // No input method on this device: nothing to edit with, so the row does nothing.
            }
        }
    }
}

private val MinTarget = 48.dp

@Composable
internal fun SettingsContent(
    state: SettingsState,
    onEditServer: () -> Unit,
    onEditAddresses: () -> Unit,
    onReset: () -> Unit,
) {
    var confirming by rememberSaveable { mutableStateOf(false) }
    val addresses = addressText(state.settings.fallback).ifEmpty { stringResource(R.string.settings_addresses_none) }
    MaterialTheme {
        val listState = rememberTransformingLazyColumnState()
        ScreenScaffold(scrollState = listState) { padding ->
            TransformingLazyColumn(state = listState, contentPadding = padding, modifier = Modifier.fillMaxSize()) {
                item { Title(stringResource(R.string.settings_title)) }
                state.error?.let { error -> item { Hint(stringResource(error.message())) } }
                if (state.saving) item { Hint(stringResource(R.string.settings_saving)) }
                item {
                    RowButton(stringResource(R.string.settings_server), displayUrl(state.settings.serverUrl), onEditServer)
                }
                item { RowButton(stringResource(R.string.settings_addresses), addresses, onEditAddresses) }
                item { Hint(stringResource(R.string.settings_help)) }
                item { RowButton(stringResource(R.string.settings_reset), onClick = { confirming = true }) }
            }
        }
        AlertDialog(
            visible = confirming,
            onDismissRequest = { confirming = false },
            confirmButton = {
                Button(onClick = { confirming = false; onReset() }, modifier = Modifier.heightIn(min = MinTarget)) {
                    Text(stringResource(R.string.settings_reset_confirm))
                }
            },
            dismissButton = {
                Button(
                    onClick = { confirming = false },
                    colors = ButtonDefaults.filledTonalButtonColors(),
                    modifier = Modifier.heightIn(min = MinTarget),
                ) { Text(stringResource(R.string.settings_cancel)) }
            },
            title = { Text(stringResource(R.string.settings_reset_title)) },
            text = { Text(stringResource(R.string.settings_reset_text)) },
        )
    }
}

private fun SettingsError.message(): Int = when (this) {
    SettingsError.BadServer -> R.string.settings_bad_server
    SettingsError.BadAddresses -> R.string.settings_bad_addresses
    SettingsError.CouldNotSave -> R.string.settings_save_failed
}

@Composable
private fun Title(text: String) {
    Box(Modifier.fillMaxWidth().padding(vertical = 4.dp), contentAlignment = Alignment.Center) {
        Text(text, color = Color.White, fontWeight = FontWeight.Bold, textAlign = TextAlign.Center)
    }
}

@Composable
private fun Hint(text: String) {
    Box(Modifier.fillMaxWidth().padding(vertical = 8.dp), contentAlignment = Alignment.Center) {
        Text(text, color = Color.White, fontSize = 13.sp, textAlign = TextAlign.Center)
    }
}

@Composable
private fun RowButton(title: String, subtitle: String = "", onClick: () -> Unit) {
    Button(onClick = onClick, modifier = Modifier.fillMaxWidth().heightIn(min = MinTarget)) {
        Column(verticalArrangement = Arrangement.Center) {
            Text(title)
            if (subtitle.isNotEmpty()) Text(subtitle, fontSize = 11.sp)
        }
    }
}
