package io.github.jfms7s.padelscore.local.engine

import io.github.jfms7s.padelscore.net.Current
import io.github.jfms7s.padelscore.net.GameScore
import io.github.jfms7s.padelscore.net.Player
import io.github.jfms7s.padelscore.net.PointCall
import io.github.jfms7s.padelscore.net.Status
import io.github.jfms7s.padelscore.net.Team
import io.github.jfms7s.padelscore.net.TieBreakScore
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class SetsMatchTest {
    private val start = Start(firstServer = Player(Team.A, 0), otherFirstServer = 0)
    private val golden = SetRules(DeuceRule.GOLDEN_POINT, SetsFormat.ONE, DecidingSet.FULL_SET)
    private val star = SetRules(DeuceRule.STAR_POINT, SetsFormat.ONE, DecidingSet.FULL_SET)
    private val advantage = SetRules(DeuceRule.ADVANTAGE, SetsFormat.ONE, DecidingSet.FULL_SET)
    private val bestOf3Super = SetRules(DeuceRule.GOLDEN_POINT, SetsFormat.BEST_OF_3, DecidingSet.SUPER_TIE_BREAK)

    private fun points(s: String): List<Team> = s.map { if (it == 'a') Team.A else Team.B }

    private fun match(rules: SetRules, s: String): SetsMatch {
        val m = checkNotNull(SetsMatch.create(rules, start))
        points(s).forEachIndexed { i, team -> assertTrue("point $i refused", m.apply(team)) }
        return m
    }

    private fun game(m: SetsMatch): GameScore = (m.score().current as Current.Game).score

    @Test
    fun golden_point_at_40_40_is_deciding_and_receiver_chooses() {
        val m = match(golden, "aaabbb")
        assertEquals(GameScore(PointCall.FORTY, PointCall.FORTY, 1, true), game(m))
        assertEquals(ServeSide.RECEIVER_CHOOSES, m.score().side)
        m.apply(Team.A)
        assertEquals(listOf(1, 0), m.score().games)
    }

    @Test
    fun star_point_decides_at_the_third_deuce() {
        val second = game(match(star, "aaabbb" + "ab"))
        assertEquals(2, second.deuces)
        assertFalse(second.decidingPoint)
        val third = game(match(star, "aaabbb" + "ab".repeat(2)))
        assertEquals(3, third.deuces)
        assertTrue(third.decidingPoint)
    }

    @Test
    fun advantage_never_has_a_deciding_point() {
        val m = match(advantage, "aaabbb" + "ab".repeat(5))
        assertFalse(game(m).decidingPoint)
        m.apply(Team.A)
        assertEquals(PointCall.ADVANTAGE, game(m).a)
    }

    @Test
    fun six_all_goes_to_a_tiebreak_to_seven() {
        val m = match(golden, "aaaabbbb".repeat(6))
        assertEquals(Current.TieBreak(TieBreakScore(0, 0, false)), m.score().current)
        points("aaaaabbbbbaa").forEach { assertTrue(m.apply(it)) }
        assertEquals(Status.Finished(Team.A), m.score().status)
        assertEquals(listOf(SetScore(7, 6, listOf(7, 5))), m.score().sets)
    }

    @Test
    fun third_set_super_tiebreak_to_ten() {
        val m = match(bestOf3Super, "aaaa".repeat(6) + "bbbb".repeat(6))
        assertEquals(Current.TieBreak(TieBreakScore(0, 0, true)), m.score().current)
        points("a".repeat(8) + "b".repeat(8) + "aa").forEach { assertTrue(m.apply(it)) }
        val score = m.score()
        assertEquals(SetScore(1, 0, listOf(10, 8)), score.sets.last())
        assertEquals(Status.Finished(Team.A), score.status)
    }

    @Test
    fun ends_change_after_odd_games() {
        val m = match(golden, "aaaa")
        assertTrue(m.score().changeEnds)
        points("bbbb").forEach { m.apply(it) }
        assertFalse(m.score().changeEnds)
    }

    @Test
    fun point_after_match_over_is_refused() {
        val m = match(golden, "aaaa".repeat(6))
        val over = m.score()
        assertEquals(Status.Finished(Team.A), over.status)
        assertFalse(m.apply(Team.B))
        assertEquals(over, m.score())
    }

    @Test
    fun one_set_super_tiebreak_is_invalid() {
        val rules = SetRules(DeuceRule.GOLDEN_POINT, SetsFormat.ONE, DecidingSet.SUPER_TIE_BREAK)
        assertFalse(rules.valid)
        assertNull(SetsMatch.create(rules, start))
        assertNull(replay(rules, start, points("a")))
    }

    @Test
    fun invalid_player_index_is_refused() {
        assertNull(SetsMatch.create(golden, Start(Player(Team.A, 2), 0)))
        assertNull(SetsMatch.create(golden, Start(Player(Team.A, 0), -1)))
    }

    @Test
    fun replay_refuses_a_point_after_the_match_is_over() {
        assertEquals(Status.Finished(Team.A), replay(golden, start, points("aaaa".repeat(6)))?.status)
        assertNull(replay(golden, start, points("aaaa".repeat(6) + "b")))
    }

    @Test
    fun to_wire_keeps_what_the_screen_reads() {
        val engine = match(golden, "aaaabba")
        val score = engine.score()
        val wire = score.toWire()
        assertEquals(score.setsWon, wire.setsWon)
        assertEquals(score.games, wire.games)
        assertEquals(score.current, wire.current)
        assertEquals(score.server, wire.server)
        assertEquals(score.changeEnds, wire.changeEnds)
        assertEquals(score.status, wire.status)
        assertEquals(score.sets.size, wire.sets?.size)
    }
}
