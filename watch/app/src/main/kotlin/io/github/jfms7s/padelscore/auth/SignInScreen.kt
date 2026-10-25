package io.github.jfms7s.padelscore.auth

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.wear.compose.material3.Button
import androidx.wear.compose.material3.MaterialTheme
import androidx.wear.compose.material3.Text
import io.github.jfms7s.padelscore.R

@Composable
fun SignInScreen(model: SignInModel, onSignedIn: () -> Unit) {
    val state by model.state.collectAsState()
    LaunchedEffect(state) {
        if (state == SignInState.SignedIn) onSignedIn()
    }
    SignInContent(state, onNewCode = model::restart)
}

@Composable
internal fun SignInContent(state: SignInState, onNewCode: () -> Unit) {
    MaterialTheme {
        Box(
            Modifier.fillMaxSize().padding(horizontal = 28.dp, vertical = 20.dp),
            contentAlignment = Alignment.Center,
        ) {
            Column(horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.Center) {
                when (state) {
                    SignInState.Starting, SignInState.SignedIn -> Plain(stringResource(R.string.sign_in_starting))
                    is SignInState.ShowCode -> Code(state)
                    SignInState.Expired -> Retry(stringResource(R.string.sign_in_expired), onNewCode)
                    SignInState.Denied -> Retry(stringResource(R.string.sign_in_denied), onNewCode)
                    is SignInState.Failed -> Retry(stringResource(state.kind.message()), onNewCode)
                }
            }
        }
    }
}

private fun FailKind.message(): Int = when (this) {
    FailKind.TooManyTries -> R.string.sign_in_too_many
    FailKind.Unreachable -> R.string.sign_in_unreachable
    FailKind.CouldNotSave -> R.string.sign_in_save_failed
}

@Composable
private fun Plain(text: String) {
    Text(text, color = Color.White, textAlign = TextAlign.Center)
}

@Composable
private fun Code(state: SignInState.ShowCode) {
    Text(stringResource(R.string.sign_in_title), color = Color.White, fontWeight = FontWeight.Bold)
    Text(
        state.userCode,
        color = Color.White,
        fontFamily = FontFamily.Monospace,
        fontWeight = FontWeight.Bold,
        fontSize = 26.sp,
        textAlign = TextAlign.Center,
    )
    Text(stringResource(R.string.sign_in_open_on_phone), color = Color.White, fontSize = 12.sp, textAlign = TextAlign.Center)
    Text(state.linkUrl, color = Color.White, fontSize = 11.sp, textAlign = TextAlign.Center)
    if (state.offline) {
        Text(stringResource(R.string.sign_in_unreachable), color = Color.White, fontSize = 12.sp, textAlign = TextAlign.Center)
    } else {
        val minutes = (state.secondsLeft + 59) / 60
        Text(stringResource(R.string.sign_in_minutes_left, minutes), color = Color.White, fontSize = 12.sp)
    }
}

@Composable
private fun Retry(message: String, onNewCode: () -> Unit) {
    Plain(message)
    Button(onClick = onNewCode, modifier = Modifier.padding(top = 8.dp)) {
        Text(stringResource(R.string.sign_in_new_code))
    }
}
