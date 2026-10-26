package io.github.jfms7s.padelscore.net

import kotlinx.serialization.SerializationException
import kotlinx.serialization.builtins.ListSerializer
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test

class WireTest {
    private fun resource(path: String): String =
        checkNotNull(javaClass.classLoader?.getResource(path)) { "missing test resource $path" }.readText()

    private fun view(name: String): MatchView = WireJson.decodeFromString(MatchView.serializer(), resource(name))

    private val phase2Views = listOf(
        "view-advantage", "view-after-undo", "view-change-ends", "view-finished", "view-golden-point-40-40",
        "view-new", "view-one-point", "view-spectator", "view-star-point", "view-super-tiebreak-finished",
        "view-super-tiebreak", "view-third-game", "view-tiebreak", "view-timed-draw", "view-timed-finished",
        "view-timed-overtime-finished", "view-timed-overtime", "view-timed", "view-undo-before", "replace-200",
    ).map { "$it.json" }

    @Test fun every_phase2_view_fixture_decodes() {
        for (name in phase2Views) {
            val raw = Json.parseToJsonElement(resource(name)).jsonObject
            val decoded = view(name)
            assertEquals(name, raw.getValue("revision").jsonPrimitive.content.toLong(), decoded.revision)
            assertEquals(name, raw.getValue("events").jsonArray.size, decoded.events.size)
        }
    }

    @Test fun tiebreak_fixture_has_tie_break_current() {
        val current = view("view-tiebreak.json").score.current
        assertEquals(Current.TieBreak(TieBreakScore(2, 1, false)), current)
    }

    @Test fun finished_fixture_has_winner() {
        assertEquals(Status.Finished(Team.A), view("view-finished.json").score.status)
    }

    @Test fun drawn_fixture_finishes_without_winner() {
        assertEquals(Status.Finished(null), view("view-timed-draw.json").score.status)
    }

    @Test fun new_match_fixture_decodes_game_score_and_last_seq() {
        val v = view("view-new.json")
        assertEquals(Current.Game(GameScore(PointCall.LOVE, PointCall.LOVE, 0, false)), v.score.current)
        assertEquals(Status.InProgress, v.score.status)
        assertNull(v.lastEffectiveSeq)
        assertEquals(24L, view("view-finished.json").lastEffectiveSeq)
    }

    @Test fun conflict_body_carries_view() {
        val body = WireJson.decodeFromString(ErrorBody.serializer(), resource("commands-409-conflict.json"))
        assertEquals("conflict", body.error)
        assertNotNull(body.view)
    }

    @Test fun rate_limit_body_carries_retry_after() {
        val body = WireJson.decodeFromString(ErrorBody.serializer(), resource("fixtures/device-token-429.json"))
        assertEquals(ErrorBody("too_many_requests", retryAfterSecs = 57), body)
    }

    @Test fun point_command_encodes_like_serde() {
        val request = CommandRequest("c1", false, 9, listOf(Command.Point(Team.A)))
        assertEquals(
            """{"client_id":"c1","batch":false,"base_revision":9,"commands":[{"kind":"point","team":"a"}]}""",
            WireJson.encodeToString(CommandRequest.serializer(), request),
        )
    }

    @Test fun undo_command_encodes_like_serde() {
        assertEquals(
            """{"kind":"undo","target_seq":17}""",
            WireJson.encodeToString(Command.serializer(), Command.Undo(17)),
        )
    }

    @Test fun phase3_fixtures_decode() {
        val start = WireJson.decodeFromString(DeviceStart.serializer(), resource("fixtures/device-start.json"))
        assertEquals("/link", start.verificationUri)
        assertEquals(5, start.interval)
        val token = WireJson.decodeFromString(DeviceToken.serializer(), resource("fixtures/device-token-200.json"))
        assertTrue(token.accessToken.startsWith("pdl_"))
        val matches = WireJson.decodeFromString(ListSerializer(MyMatch.serializer()), resource("fixtures/me-matches.json"))
        assertEquals(1, matches.size)
        assertEquals("Ana / Rui v João / Marta", matches[0].label)
        assertNotNull(matches[0].view)
    }

    @Test fun current_and_status_round_trip() {
        for (current in listOf(Current.None, Current.Game(GameScore(PointCall.FORTY, PointCall.ADVANTAGE, 2, true)), Current.TieBreak(TieBreakScore(3, 4, true)))) {
            val text = WireJson.encodeToString(Current.serializer(), current)
            assertEquals(current, WireJson.decodeFromString(Current.serializer(), text))
        }
        for (status in listOf(Status.InProgress, Status.Finished(Team.B), Status.Finished(null))) {
            val text = WireJson.encodeToString(Status.serializer(), status)
            assertEquals(status, WireJson.decodeFromString(Status.serializer(), text))
        }
    }

    @Test fun unknown_current_tag_fails() {
        try {
            WireJson.decodeFromString(Current.serializer(), """{"penalty":{}}""")
            fail("expected SerializationException")
        } catch (_: SerializationException) {
        }
    }

    @Test fun unknown_status_tag_fails() {
        try {
            WireJson.decodeFromString(Status.serializer(), "\"paused\"")
            fail("expected SerializationException")
        } catch (_: SerializationException) {
        }
    }

    @Test fun device_token_to_string_hides_the_token() {
        val token = WireJson.decodeFromString(DeviceToken.serializer(), resource("fixtures/device-token-200.json"))
        assertFalse(token.toString().contains(token.accessToken))
        assertEquals("DeviceToken(accessToken=██)", token.toString())
    }
}
