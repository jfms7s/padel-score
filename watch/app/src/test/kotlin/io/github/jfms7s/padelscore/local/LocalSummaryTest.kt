package io.github.jfms7s.padelscore.local

import io.github.jfms7s.padelscore.local.engine.DecidingSet
import io.github.jfms7s.padelscore.local.engine.DeuceRule
import io.github.jfms7s.padelscore.local.engine.EngineScore
import io.github.jfms7s.padelscore.local.engine.SetRules
import io.github.jfms7s.padelscore.local.engine.SetsFormat
import io.github.jfms7s.padelscore.local.engine.Start
import io.github.jfms7s.padelscore.local.engine.replay
import io.github.jfms7s.padelscore.net.Player
import io.github.jfms7s.padelscore.net.Team
import org.junit.Assert.assertEquals
import org.junit.Test

class LocalSummaryTest {
    private val start = Start(firstServer = Player(Team.A, 0), otherFirstServer = 0)
    private val bestOf3 = SetRules(DeuceRule.GOLDEN_POINT, SetsFormat.BEST_OF_3, DecidingSet.FULL_SET)
    private val bestOf3Super = SetRules(DeuceRule.GOLDEN_POINT, SetsFormat.BEST_OF_3, DecidingSet.SUPER_TIE_BREAK)
    private val advantage = SetRules(DeuceRule.ADVANTAGE, SetsFormat.ONE, DecidingSet.FULL_SET)

    /** Each letter is a game won to love by that team. */
    private fun games(s: String): String = s.map { it.toString().repeat(4) }.joinToString("")

    private fun score(rules: SetRules, points: String): EngineScore =
        checkNotNull(replay(rules, start, points.map { if (it == 'a') Team.A else Team.B }))

    private val firstSet64 = games("bbbbaaaaaa")

    @Test
    fun no_points() = assertEquals("0-0 · 0-0", summary(score(bestOf3, "")))

    @Test
    fun game_in_progress() = assertEquals("0-0 · 30-15", summary(score(bestOf3, "aab")))

    @Test
    fun second_set_in_progress() =
        assertEquals("6-4 · 3-2 · 30-15", summary(score(bestOf3, firstSet64 + games("aaabb") + "aab")))

    @Test
    fun regular_tie_break() =
        assertEquals("6-4 · 6-6 · 5-3", summary(score(bestOf3, firstSet64 + games("aaaaabbbbbba") + "aaaaabbb")))

    @Test
    fun advantage_is_ad() = assertEquals("0-0 · AD-40", summary(score(advantage, "aaabbba")))

    @Test
    fun super_tie_break_in_progress_leaves_out_its_set() = assertEquals(
        "6-4 · 4-6 · 7-5",
        summary(score(bestOf3Super, firstSet64 + games("aaaabbbbbb") + "aaaaaaabbbbb")),
    )

    @Test
    fun finished_with_super_tie_break_shows_its_points() = assertEquals(
        "6-4 · 4-6 · 10-8",
        summary(score(bestOf3Super, firstSet64 + games("aaaabbbbbb") + "bbbbbbbb" + "aaaaaaaaaa")),
    )

    @Test
    fun finished_in_two_sets() =
        assertEquals("6-4 · 6-3", summary(score(bestOf3, firstSet64 + games("bbbaaaaaa"))))
}
