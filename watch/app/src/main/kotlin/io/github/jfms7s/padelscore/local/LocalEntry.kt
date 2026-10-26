package io.github.jfms7s.padelscore.local

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.wear.compose.material3.AlertDialog
import androidx.wear.compose.material3.Button
import androidx.wear.compose.material3.ButtonDefaults
import androidx.wear.compose.material3.Text
import io.github.jfms7s.padelscore.R
import io.github.jfms7s.padelscore.score.Feedback

private val MinTarget = 48.dp

/**
 * The local match buttons for the bottom of a first screen: start one, or resume or discard the
 * one in progress. Nothing until the store has loaded. A Column, so a screen puts it in one item.
 */
@Composable
fun LocalEntryButtons(ui: LocalUi, onNew: () -> Unit, onResume: () -> Unit, onDiscard: () -> Unit) {
    if (!ui.loaded) return
    var confirming by rememberSaveable { mutableStateOf(false) }
    // Only a discard writes from here, so a new Not saved is a discard that failed.
    val opened = remember { ui.feedbackId }
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
            if (ui.feedbackSince(opened) == Feedback.NotSaved) {
                Text(
                    stringResource(R.string.local_not_saved),
                    color = Color.White,
                    fontSize = 13.sp,
                    textAlign = TextAlign.Center,
                    modifier = Modifier.fillMaxWidth(),
                )
            }
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
