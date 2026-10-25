package io.github.jfms7s.padelscore.net

import kotlinx.serialization.KSerializer
import kotlinx.serialization.SerializationException
import kotlinx.serialization.builtins.serializer
import kotlinx.serialization.descriptors.SerialDescriptor
import kotlinx.serialization.encoding.Decoder
import kotlinx.serialization.encoding.Encoder
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonDecoder
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonEncoder
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.decodeFromJsonElement
import kotlinx.serialization.json.encodeToJsonElement
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.put

// serde's default enum encoding: a unit variant is a bare string, any other is {"variant": payload}.
// Kotlin has no built-in equivalent, so each enum below decodes through JsonElement by hand.

private fun JsonDecoder.decodeTagged(what: String): Pair<String, JsonElement?> =
    when (val element = decodeJsonElement()) {
        is JsonPrimitive -> if (element.isString) element.content to null else fail(what, element)
        is JsonObject -> element.entries.singleOrNull()?.let { it.key to it.value } ?: fail(what, element)
        else -> fail(what, element)
    }

private fun fail(what: String, element: JsonElement): Nothing =
    throw SerializationException("Cannot decode $what from $element")

private fun Decoder.json(): JsonDecoder =
    this as? JsonDecoder ?: throw SerializationException("$this is not a JSON decoder")

private fun Encoder.json(): JsonEncoder =
    this as? JsonEncoder ?: throw SerializationException("$this is not a JSON encoder")

internal object CurrentSerializer : KSerializer<Current> {
    override val descriptor: SerialDescriptor = JsonElement.serializer().descriptor

    override fun deserialize(decoder: Decoder): Current {
        val json = decoder.json()
        val (tag, payload) = json.decodeTagged("Current")
        return when {
            tag == "none" && payload == null -> Current.None
            tag == "game" && payload != null ->
                Current.Game(json.json.decodeFromJsonElement(GameScore.serializer(), payload))
            tag == "tie_break" && payload != null ->
                Current.TieBreak(json.json.decodeFromJsonElement(TieBreakScore.serializer(), payload))
            else -> throw SerializationException("Unknown Current variant '$tag'")
        }
    }

    override fun serialize(encoder: Encoder, value: Current) {
        val json = encoder.json()
        json.encodeJsonElement(
            when (value) {
                Current.None -> JsonPrimitive("none")
                is Current.Game -> buildJsonObject {
                    put("game", json.json.encodeToJsonElement(GameScore.serializer(), value.score))
                }
                is Current.TieBreak -> buildJsonObject {
                    put("tie_break", json.json.encodeToJsonElement(TieBreakScore.serializer(), value.score))
                }
            },
        )
    }
}

internal object StatusSerializer : KSerializer<Status> {
    override val descriptor: SerialDescriptor = JsonElement.serializer().descriptor

    override fun deserialize(decoder: Decoder): Status {
        val json = decoder.json()
        val (tag, payload) = json.decodeTagged("Status")
        return when {
            tag == "in_progress" && payload == null -> Status.InProgress
            tag == "finished" && payload != null -> {
                val winner = payload.jsonObject["winner"]?.takeUnless { it is JsonNull }
                Status.Finished(winner?.let { json.json.decodeFromJsonElement(Team.serializer(), it) })
            }
            else -> throw SerializationException("Unknown Status variant '$tag'")
        }
    }

    override fun serialize(encoder: Encoder, value: Status) {
        val json = encoder.json()
        json.encodeJsonElement(
            when (value) {
                Status.InProgress -> JsonPrimitive("in_progress")
                is Status.Finished -> buildJsonObject {
                    put("finished", buildJsonObject {
                        put("winner", value.winner?.let { json.json.encodeToJsonElement(Team.serializer(), it) } ?: JsonNull)
                    })
                }
            },
        )
    }
}
