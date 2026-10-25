// AGP 9 compiles Kotlin itself; the Kotlin plugins below only add the Compose compiler and
// serialization, and pin the Kotlin Gradle plugin version for the whole build.
plugins {
    alias(libs.plugins.android.application) apply false
    alias(libs.plugins.kotlin.compose) apply false
    alias(libs.plugins.kotlin.serialization) apply false
}
