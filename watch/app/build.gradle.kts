plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.kotlin.compose)
    alias(libs.plugins.kotlin.serialization)
}

// CI signs with the owner's key from GitHub secrets so each build installs over the last
// (and keeps the sign-in). Without all four variables, builds use the machine's debug key.
val ciSigningEnv = listOf("WATCH_KEYSTORE_PATH", "WATCH_KEYSTORE_PASSWORD", "WATCH_KEY_ALIAS", "WATCH_KEY_PASSWORD")
    .associateWith { providers.environmentVariable(it).orNull?.takeIf(String::isNotEmpty) }
val hasCiSigning = ciSigningEnv.values.all { it != null }

val serverUrl = providers.gradleProperty("padelServerUrl").get()
val serverFallbackIps = providers.gradleProperty("padelServerFallbackIps").orElse("").get()

android {
    namespace = "io.github.jfms7s.padelscore"
    compileSdk = 37

    defaultConfig {
        applicationId = "io.github.jfms7s.padelscore.watch"
        minSdk = 30
        targetSdk = 36
        versionCode = 1
        versionName = "0.1.0"
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
        buildConfigField("String", "SERVER_URL", "\"$serverUrl\"")
        buildConfigField("String", "SERVER_FALLBACK_IPS", "\"$serverFallbackIps\"")
    }

    signingConfigs {
        if (hasCiSigning) {
            create("ci") {
                storeFile = file(ciSigningEnv.getValue("WATCH_KEYSTORE_PATH")!!)
                storePassword = ciSigningEnv.getValue("WATCH_KEYSTORE_PASSWORD")
                keyAlias = ciSigningEnv.getValue("WATCH_KEY_ALIAS")
                keyPassword = ciSigningEnv.getValue("WATCH_KEY_PASSWORD")
            }
        }
    }

    buildTypes {
        debug {
            if (hasCiSigning) signingConfig = signingConfigs.getByName("ci")
        }
    }

    buildFeatures {
        compose = true
        buildConfig = true
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_21
        targetCompatibility = JavaVersion.VERSION_21
    }

    sourceSets {
        // The web's serde fixtures, so the watch decodes the same bytes the web is tested against.
        getByName("test").resources.directories.add("../../web/src/lib/fixtures")
    }

    testOptions {
        unitTests.isIncludeAndroidResources = true
        unitTests.all {
            // Robolectric's SDK 36 sandbox reaches into FileDescriptor internals, which JDK 21 hides.
            it.jvmArgs("--add-exports=java.base/jdk.internal.access=ALL-UNNAMED")
        }
    }
}

kotlin {
    jvmToolchain(21)
}

dependencies {
    val composeBom = platform(libs.compose.bom)
    implementation(composeBom)
    implementation(libs.wear.compose.material3)
    implementation(libs.wear.compose.foundation)
    implementation(libs.wear.compose.navigation)
    implementation(libs.lifecycle.viewmodel.compose)
    implementation(libs.lifecycle.runtime.compose)
    implementation(libs.activity.compose)
    implementation(libs.core.ktx)
    implementation(libs.kotlinx.serialization.json)
    implementation(libs.kotlinx.coroutines.android)
    implementation(libs.datastore.preferences)
    implementation(libs.wear.input)
    implementation(libs.okhttp)
    implementation(libs.okhttp.sse)
    implementation(libs.compose.ui.tooling.preview)
    implementation(libs.wear.tooling.preview)
    debugImplementation(libs.compose.ui.tooling)
    // Hosts createComposeRule()'s activity in Robolectric tests.
    debugImplementation(libs.compose.ui.test.manifest)

    testImplementation(composeBom)
    testImplementation(libs.junit)
    testImplementation(libs.robolectric)
    testImplementation(libs.androidx.test.core)
    testImplementation(libs.androidx.test.ext.junit)
    testImplementation(libs.compose.ui.test.junit4)
    testImplementation(libs.mockwebserver3)
    testImplementation(libs.kotlinx.coroutines.test)

    androidTestImplementation(libs.junit)
    androidTestImplementation(libs.androidx.test.ext.junit)
    androidTestImplementation(libs.androidx.test.runner)
    androidTestImplementation(libs.kotlinx.coroutines.test)
}
