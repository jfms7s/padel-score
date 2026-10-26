package io.github.jfms7s.padelscore.local

import io.github.jfms7s.padelscore.haptics.Haptics

/** What the local match needs; built once per process, with or without a server set. */
data class LocalDeps(val store: LocalMatchStore, val haptics: Haptics)
