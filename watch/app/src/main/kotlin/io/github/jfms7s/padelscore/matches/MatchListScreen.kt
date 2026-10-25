package io.github.jfms7s.padelscore.matches

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.wear.compose.foundation.lazy.TransformingLazyColumn
import androidx.wear.compose.foundation.lazy.items
import androidx.wear.compose.foundation.lazy.rememberTransformingLazyColumnState
import androidx.wear.compose.material3.AlertDialog
import androidx.wear.compose.material3.Button
import androidx.wear.compose.material3.ButtonDefaults
import androidx.wear.compose.material3.MaterialTheme
import androidx.wear.compose.material3.ScreenScaffold
import androidx.wear.compose.material3.Text
import io.github.jfms7s.padelscore.R

@Composable
fun MatchListScreen(model: MatchListModel, onPick: (String) -> Unit, onSignOut: () -> Unit) {
    val state by model.state.collectAsState()
    MatchListContent(state, onPick, onRetry = model::refresh, onSignOut = onSignOut)
}

private val MinTarget = 48.dp

@Composable
internal fun MatchListContent(
    state: ListState,
    onPick: (String) -> Unit,
    onRetry: () -> Unit,
    onSignOut: () -> Unit,
) {
    var confirming by rememberSaveable { mutableStateOf(false) }
    MaterialTheme {
        val listState = rememberTransformingLazyColumnState()
        ScreenScaffold(scrollState = listState) { padding ->
            TransformingLazyColumn(state = listState, contentPadding = padding, modifier = Modifier.fillMaxSize()) {
                when (state) {
                    ListState.Loading, ListState.Unauthorized -> item { Hint(stringResource(R.string.matches_loading)) }
                    is ListState.Failed -> {
                        item { Hint(stringResource(state.kind.message())) }
                        item { RowButton(stringResource(R.string.matches_retry), onClick = onRetry) }
                    }
                    is ListState.Ready -> {
                        if (!state.live) item { Hint(stringResource(R.string.matches_offline)) }
                        if (state.rows.isEmpty()) item { Hint(stringResource(R.string.matches_empty)) }
                        items(state.rows, key = { it.id }) { row ->
                            RowButton(row.title, subtitle = row.subtitle, onClick = { onPick(row.id) })
                        }
                    }
                }
                item { RowButton(stringResource(R.string.sign_out), onClick = { confirming = true }) }
            }
        }
        AlertDialog(
            visible = confirming,
            onDismissRequest = { confirming = false },
            confirmButton = {
                Button(onClick = { confirming = false; onSignOut() }, modifier = Modifier.heightIn(min = MinTarget)) {
                    Text(stringResource(R.string.sign_out_confirm))
                }
            },
            dismissButton = {
                Button(
                    onClick = { confirming = false },
                    colors = ButtonDefaults.filledTonalButtonColors(),
                    modifier = Modifier.heightIn(min = MinTarget),
                ) { Text(stringResource(R.string.sign_out_cancel)) }
            },
            title = { Text(stringResource(R.string.sign_out_title)) },
            text = { Text(stringResource(R.string.sign_out_text)) },
        )
    }
}

private fun ListFailKind.message(): Int = when (this) {
    ListFailKind.TooManyTries -> R.string.matches_too_many
    ListFailKind.Unreachable -> R.string.matches_unreachable
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
