package io.github.jfms7s.padelscore.local

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.wear.compose.material3.AlertDialog
import androidx.wear.compose.material3.Button
import androidx.wear.compose.material3.ButtonDefaults
import androidx.wear.compose.material3.Text
import io.github.jfms7s.padelscore.R

private val MinTarget = 48.dp

/**
 * The local match buttons for the bottom of a first screen: start one, or resume or discard the
 * one in progress. Nothing until the store has loaded. A Column, so a screen puts it in one item.
 */
@Composable
fun LocalEntryButtons(ui: LocalUi, onNew: () -> Unit, onResume: () -> Unit, onDiscard: () -> Unit) {
    if (!ui.loaded) return
    var confirming by rememberSaveable { mutableStateOf(false) }
    val score = ui.score
    Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
        if (ui.match == null || score == null) {
            Button(onClick = onNew, modifier = Modifier.fillMaxWidth().heightIn(min = MinTarget)) {
                Text(stringResource(R.string.local_match))
            }
        } else {
            Button(onClick = onResume, modifier = Modifier.fillMaxWidth().heightIn(min = MinTarget)) {
                Column {
                    Text(stringResource(R.string.local_resume))
                    Text(summary(score), fontSize = 11.sp)
                }
            }
            Button(
                onClick = { confirming = true },
                colors = ButtonDefaults.filledTonalButtonColors(),
                modifier = Modifier.fillMaxWidth().heightIn(min = MinTarget),
            ) { Text(stringResource(R.string.local_discard)) }
        }
    }
    AlertDialog(
        visible = confirming,
        onDismissRequest = { confirming = false },
        confirmButton = {
            Button(onClick = { confirming = false; onDiscard() }, modifier = Modifier.heightIn(min = MinTarget)) {
                Text(stringResource(R.string.local_discard_confirm))
            }
        },
        dismissButton = {
            Button(
                onClick = { confirming = false },
                colors = ButtonDefaults.filledTonalButtonColors(),
                modifier = Modifier.heightIn(min = MinTarget),
            ) { Text(stringResource(R.string.local_discard_cancel)) }
        },
        title = { Text(stringResource(R.string.local_discard_title)) },
    )
}
