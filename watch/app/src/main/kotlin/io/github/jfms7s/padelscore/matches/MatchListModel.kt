package io.github.jfms7s.padelscore.matches

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import io.github.jfms7s.padelscore.net.ApiResult
import io.github.jfms7s.padelscore.net.MyMatch
import io.github.jfms7s.padelscore.net.PadelApi
import io.github.jfms7s.padelscore.net.StreamItem
import kotlinx.coroutines.Job
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch

data class MatchRow(val id: String, val title: String, val subtitle: String)

/** Why the first load failed; the screen maps each to a string resource. */
enum class ListFailKind { TooManyTries, Unreachable }

sealed interface ListState {
    data object Loading : ListState
    data class Ready(val rows: List<MatchRow>, val live: Boolean) : ListState
    data class Failed(val kind: ListFailKind) : ListState
    data object Unauthorized : ListState
}

/**
 * Today every row is a quick match, so the title is fixed and the subtitle says who plays.
 * The server's label wins over names built from the view.
 */
fun rowOf(m: MyMatch, quickMatchTitle: String): MatchRow {
    val teams = m.view?.teams
    val subtitle = m.label
        ?: teams?.let { "${it.a.joinToString(" / ")} v ${it.b.joinToString(" / ")}" }
        ?: ""
    return MatchRow(m.id, quickMatchTitle, subtitle)
}

/**
 * [stream] is collected once, from construction: the server keeps at most 5 list streams per user.
 * The stream is the source of truth, so a frame always wins over the one-off GET, even if the GET
 * answers later.
 */
class MatchListModel(
    private val api: PadelApi,
    stream: Flow<StreamItem<List<MyMatch>>>,
    private val quickMatchTitle: String,
) : ViewModel() {
    private val _state = MutableStateFlow<ListState>(ListState.Loading)
    val state: StateFlow<ListState> = _state.asStateFlow()

    private var streamFilled = false
    private var live = true
    private var load: Job? = null

    init {
        viewModelScope.launch { stream.collect(::onStream) }
        refresh()
    }

    fun refresh() {
        load?.cancel()
        load = viewModelScope.launch { onGet(api.myMatches()) }
    }

    private fun onGet(result: ApiResult<List<MyMatch>>) {
        if (_state.value == ListState.Unauthorized) return
        when (result) {
            ApiResult.Unauthorized -> _state.value = ListState.Unauthorized
            is ApiResult.Ok -> if (!streamFilled) show(result.value)
            is ApiResult.TooMany -> fail(ListFailKind.TooManyTries)
            is ApiResult.Failed -> fail(ListFailKind.Unreachable)
        }
    }

    private fun fail(kind: ListFailKind) {
        if (!streamFilled) _state.value = ListState.Failed(kind)
    }

    private fun onStream(item: StreamItem<List<MyMatch>>) {
        if (_state.value == ListState.Unauthorized) return
        when (item) {
            is StreamItem.Frame -> {
                streamFilled = true
                live = true
                show(item.value)
            }
            StreamItem.Live -> setLive(true)
            StreamItem.Offline -> setLive(false)
            StreamItem.Unauthorized -> _state.value = ListState.Unauthorized
        }
    }

    private fun show(matches: List<MyMatch>) {
        _state.value = ListState.Ready(matches.map { rowOf(it, quickMatchTitle) }, live)
    }

    private fun setLive(value: Boolean) {
        live = value
        (_state.value as? ListState.Ready)?.let { _state.value = it.copy(live = value) }
    }
}
