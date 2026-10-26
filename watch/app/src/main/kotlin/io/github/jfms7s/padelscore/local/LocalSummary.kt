package io.github.jfms7s.padelscore.local

import io.github.jfms7s.padelscore.local.engine.EngineScore
import io.github.jfms7s.padelscore.local.engine.SetScore
import io.github.jfms7s.padelscore.net.Current
import io.github.jfms7s.padelscore.score.callLabel

/**
 * One line for a local match's score, "6-4 · 3-2 · 30-15": each set, then the current points.
 * A super tie-break set shows its points once won, and is left out while in progress (its
 * points are the current ones). A finished match shows only its sets.
 */
fun summary(score: EngineScore): String {
    val sets = score.sets.mapNotNull(::setLabel)
    val current = when (val c = score.current) {
        is Current.Game -> "${callLabel(c.score.a)}-${callLabel(c.score.b)}"
        is Current.TieBreak -> "${c.score.a}-${c.score.b}"
        Current.None -> null
    }
    return (sets + listOfNotNull(current)).joinToString(" · ")
}

// A super tie-break set counts as one game: 1-0 or 0-1 once won, 0-0 while it is played.
private fun setLabel(set: SetScore): String? {
    val tiebreak = set.tiebreak
    val games = set.a + set.b
    if (tiebreak == null || games > 1) return "${set.a}-${set.b}"
    return if (games == 1) tiebreak.joinToString("-") else null
}
