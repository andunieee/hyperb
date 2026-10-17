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
        versionCode = 1
        versionName = "0.1.0"
        ndk {
            // Rust cdylib built via: cargo ndk -t arm64-v8a -t armeabi-v7a -o app/src/main/jniLibs build -p hyperb-mobile
            abiFilters += listOf("arm64-v8a", "armeabi-v7a", "x86_64")
        }
    }

    buildTypes {
        release {
            isMinifyEnabled = false
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
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-android:1.9.0")
    implementation("org.rustls:rustls-platform-verifier")
    implementation("androidx.compose.ui:ui-tooling-preview")
    debugImplementation("androidx.compose.ui:ui-tooling")
}
