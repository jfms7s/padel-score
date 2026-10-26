package io.github.jfms7s.padelscore.local.engine

import io.github.jfms7s.padelscore.net.Team
import io.github.jfms7s.padelscore.net.WireJson
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

@Serializable
private data class EngineVector(
    val name: String,
    val rules: SetRules,
    val start: Start,
    val events: String,
    val steps: List<JsonElement>,
)

@Serializable
private data class EngineVectors(val vectors: List<EngineVector>)

// The Rust engine's own outputs (crates/scoring/tests/engine_vectors.rs): the port must match every step.
class EngineVectorsTest {
    private val vectors: List<EngineVector> by lazy {
        val text = checkNotNull(javaClass.classLoader?.getResource("engine-vectors.json")) {
            "missing test resource engine-vectors.json"
        }.readText()
        WireJson.decodeFromString(EngineVectors.serializer(), text).vectors
    }

    @Test
    fun port_matches_every_engine_vector() {
        assertTrue(vectors.isNotEmpty())
        for (v in vectors) {
            val m = checkNotNull(SetsMatch.create(v.rules, v.start)) { "${v.name}: rejected rules or start" }
            assertEquals("${v.name}: one step per event", v.events.length, v.steps.size)
            v.events.forEachIndexed { i, c ->
                val team = if (c == 'a') Team.A else Team.B
                val step = v.steps[i]
                if (step is JsonObject && "error" in step) {
                    val before = m.score()
                    assertFalse("${v.name}[$i]", m.apply(team))
                    assertEquals("${v.name}[$i]", before, m.score())
                } else {
                    assertTrue("${v.name}[$i]", m.apply(team))
                    assertEquals("${v.name}[$i]", WireJson.decodeFromJsonElement(EngineScore.serializer(), step), m.score())
                }
            }
        }
    }

    @Test
    fun vectors_cover_every_rule_set_and_start() {
        assertEquals(9, vectors.map { it.rules }.distinct().size)
        assertEquals(8, vectors.map { it.start }.distinct().size)
    }
}
