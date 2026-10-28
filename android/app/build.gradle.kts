plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
    id("org.jetbrains.kotlin.plugin.compose")
}

/**
 * The version of rustls-platform-verifier-android in Cargo.lock: its Kotlin
 * component must match the Rust crate exactly.
 */
abstract class RustlsVersion : ValueSource<String, RustlsVersion.Params> {
    interface Params : ValueSourceParameters {
        val lockFile: RegularFileProperty
    }

    override fun obtain(): String {
        val lines = parameters.lockFile.get().asFile.readLines()
        val i = lines.indexOfFirst { it.trim() == "name = \"rustls-platform-verifier-android\"" }
        return lines.drop(i + 1).takeIf { i >= 0 }
            ?.firstOrNull { it.trimStart().startsWith("version = ") }
            ?.substringAfter('"')?.substringBefore('"')
            ?: error("rustls-platform-verifier-android not found in Cargo.lock")
    }
}

val rustlsVersion = providers.of(RustlsVersion::class.java) {
    parameters.lockFile.set(rootProject.layout.projectDirectory.file("../Cargo.lock"))
}

configurations.configureEach {
    resolutionStrategy.eachDependency {
        if (requested.group == "org.rustls" && requested.name == "rustls-platform-verifier") {
            useVersion(rustlsVersion.get())
            because("must match the rustls-platform-verifier-android crate")
        }
    }
}

android {
    namespace = "com.hyperb.app"
    compileSdk = 35

    defaultConfig {
        applicationId = "com.hyperb.app"
        minSdk = 26
        targetSdk = 35
        // The release workflow derives these from an android-<N>-<M> tag.
        versionCode = System.getenv("ANDROID_VERSION_CODE")?.toInt() ?: 1
        versionName = System.getenv("ANDROID_VERSION_NAME") ?: "0.1.0"
        ndk {
            // Rust cdylib built via: cargo ndk -t arm64-v8a -t armeabi-v7a -o app/src/main/jniLibs build -p hyperb-mobile
            abiFilters += listOf("arm64-v8a", "armeabi-v7a", "x86_64")
        }
    }

    signingConfigs {
        // Only set up by the release workflow; locally, release builds fall back to the debug key.
        System.getenv("ANDROID_KEYSTORE")?.let { keystore ->
            create("release") {
                storeFile = file(keystore)
                storePassword = System.getenv("ANDROID_KEYSTORE_PASSWORD")
                keyAlias = System.getenv("ANDROID_KEY_ALIAS")
                keyPassword = System.getenv("ANDROID_KEY_PASSWORD")
            }
        }
    }

    buildTypes {
        release {
            isMinifyEnabled = false
            signingConfig = signingConfigs.findByName("release") ?: signingConfigs.getByName("debug")
        }
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    buildFeatures {
        compose = true
    }
}

kotlin {
    compilerOptions {
        jvmTarget.set(org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_17)
    }
}

dependencies {
    val composeBom = platform("androidx.compose:compose-bom:2025.01.00")
    implementation(composeBom)
    androidTestImplementation(composeBom)
    implementation("androidx.core:core-ktx:1.15.0")
    implementation("androidx.activity:activity-compose:1.10.1")
    implementation("androidx.compose.ui:ui")
    implementation("androidx.compose.material3:material3")
    implementation("androidx.compose.material:material-icons-core")
    // Generates the color schemes from a seed color, with Google's material-color-utilities.
    implementation("com.materialkolor:material-kolor:2.0.2")
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-android:1.9.0")
    implementation("org.rustls:rustls-platform-verifier")
    // The QR scanner used when no scanner app (Binary Eye, ...) is installed.
    implementation("com.journeyapps:zxing-android-embedded:4.3.0")
    implementation("androidx.compose.ui:ui-tooling-preview")
    debugImplementation("androidx.compose.ui:ui-tooling")
}
