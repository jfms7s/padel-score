package io.github.jfms7s.padelscore.score

import io.github.jfms7s.padelscore.net.Current
import io.github.jfms7s.padelscore.net.GameScore
import io.github.jfms7s.padelscore.net.MatchView
import io.github.jfms7s.padelscore.net.PointCall
import io.github.jfms7s.padelscore.net.Team
import io.github.jfms7s.padelscore.net.WireJson
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/** A web serde fixture from the test classpath (see the `test` source set in build.gradle.kts). */
fun fixtureView(name: String): MatchView {
    val text = checkNotNull(ScoreLabelsTest::class.java.classLoader?.getResource(name)) { "missing $name" }.readText()
    return WireJson.decodeFromString(MatchView.serializer(), text)
}

class ScoreLabelsTest {
    private fun withGame(a: PointCall, b: PointCall) = fixtureView("view-new.json").score.let {
        it.copy(current = Current.Game(GameScore(a, b, 0, false)))
    }

    @Test fun each_point_call_has_its_label() {
        val expected = mapOf(
            PointCall.LOVE to "0",
            PointCall.FIFTEEN to "15",
            PointCall.THIRTY to "30",
            PointCall.FORTY to "40",
            PointCall.ADVANTAGE to "AD",
        )
        for ((call, label) in expected) {
            assertEquals(label, pointsLabel(withGame(call, PointCall.LOVE), Team.A))
            assertEquals(label, pointsLabel(withGame(PointCall.LOVE, call), Team.B))
        }
        val advantage = fixtureView("view-advantage.json").score
        assertEquals("AD", pointsLabel(advantage, Team.A))
        assertEquals("40", pointsLabel(advantage, Team.B))
    }

    @Test fun tie_break_shows_point_numbers() {
        val score = fixtureView("view-tiebreak.json").score
        assertEquals("2", pointsLabel(score, Team.A))
        assertEquals("1", pointsLabel(score, Team.B))
    }

    @Test fun no_current_game_shows_no_points() {
        assertEquals("", pointsLabel(fixtureView("view-finished.json").score, Team.A))
    }

    @Test fun games_and_sets_label_has_g_s_shape() {
        val score = fixtureView("view-finished.json").score
        assertEquals("G6 S1", gamesSetsLabel(score, Team.A))
        assertEquals("G0 S0", gamesSetsLabel(score, Team.B))
        assertEquals("G6 S0", gamesSetsLabel(fixtureView("view-tiebreak.json").score, Team.B))
    }

    @Test fun timed_match_shows_games_only() {
        val score = fixtureView("view-timed.json").score
        assertEquals("G1", gamesSetsLabel(score, Team.A))
        assertEquals("G0", gamesSetsLabel(score, Team.B))
        assertEquals("G1", gamesSetsLabel(fixtureView("view-timed-finished.json").score, Team.A))
    }

    @Test fun team_label_joins_player_names() {
        val teams = fixtureView("view-new.json").teams
        assertEquals("Ana / Rui", teamLabel(teams, Team.A))
        assertEquals("João / Marta", teamLabel(teams, Team.B))
    }

    @Test fun serving_follows_server_and_stops_when_finished() {
        val timed = fixtureView("view-timed.json").score
        assertTrue(isServing(timed, Team.B))
        assertFalse(isServing(timed, Team.A))
        val finished = fixtureView("view-finished.json").score
        assertFalse(isServing(finished, Team.A))
        assertFalse(isServing(finished, Team.B))
    }

    @Test fun deciding_point_only_at_golden_point() {
        assertTrue(decidingPoint(fixtureView("view-golden-point-40-40.json").score))
        assertFalse(decidingPoint(fixtureView("view-advantage.json").score))
        assertFalse(decidingPoint(fixtureView("view-tiebreak.json").score))
        assertFalse(decidingPoint(fixtureView("view-finished.json").score))
    }

    @Test fun golden_point_comes_from_the_match_format() {
        assertTrue(goldenPoint(fixtureView("view-golden-point-40-40.json")))
        assertTrue(goldenPoint(fixtureView("view-timed.json")))
        assertFalse(goldenPoint(fixtureView("view-star-point.json")))
        assertFalse(goldenPoint(fixtureView("view-advantage.json").copy(format = null)))
    }
}
