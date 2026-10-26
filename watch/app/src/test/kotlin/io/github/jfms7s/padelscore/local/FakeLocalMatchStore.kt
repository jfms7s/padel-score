package io.github.jfms7s.padelscore.local

import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableStateFlow
import java.io.IOException

class FakeLocalMatchStore(initial: LocalStoreState = LocalStoreState(null, null)) : LocalMatchStore {
    private val current = MutableStateFlow(initial)
    override val state: Flow<LocalStoreState> = current

    /** Makes every write throw [IOException]. */
    var failWrites = false

    /** When set, a write waits for it to complete before applying, to hold a write open. */
    var gate: CompletableDeferred<Unit>? = null

    /** Every match saved, in the order the writes applied. */
    val saved = mutableListOf<LocalMatch>()

    override suspend fun save(match: LocalMatch) {
        gate?.await()
        if (failWrites) throw IOException("write failed")
        saved += match
        current.value = LocalStoreState(match, match.setup)
    }

    override suspend fun clear() {
        gate?.await()
        if (failWrites) throw IOException("write failed")
        current.value = current.value.copy(match = null)
    }
}
