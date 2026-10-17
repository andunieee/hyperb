# List recipes
export ANDROID_HOME := env_var_or_default('ANDROID_HOME', '/opt/android-sdk')
export ANDROID_NDK_HOME := env_var_or_default('ANDROID_NDK_HOME', ANDROID_HOME / 'ndk' / '26.3.11579264')

default:
    @just --list

# One-time tool setup (cargo-leptos, wasm and android targets, cargo-ndk)
setup:
    cargo install cargo-leptos --locked
    cargo install cargo-ndk --locked
    rustup target add wasm32-unknown-unknown aarch64-linux-android armv7-linux-androideabi

# Type-check the whole workspace
check:
    cargo check --workspace

# Run all tests
test:
    cargo test --workspace

# Lint the whole workspace
clippy:
    cargo clippy --workspace --all-targets

# Format all Rust code
fmt:
    cargo fmt --all

# Build every native crate
build:
    cargo build --workspace

# Run the GTK4 desktop app
desktop *args:
    cargo run -p hyperb-desktop -- {{args}}

# Run the terminal app
tui *args:
    cargo run -p hyperb-tui -- {{args}}

# Serve the hypereffect demo apps over HTTP + SSE (default 127.0.0.1:9001)
devserver addr="127.0.0.1:9001":
    cargo run -p hyperb-devserver -- {{addr}}

# Run the devserver and the desktop app together
dev:
    #!/usr/bin/env bash
    set -euo pipefail
    cargo build -p hyperb-devserver -p hyperb-desktop
    cargo run -q -p hyperb-devserver &
    trap 'kill $!' EXIT
    cargo run -q -p hyperb-desktop

# Leptos web app with hot reload on http://127.0.0.1:3000
[working-directory: 'crates/web']
web:
    cargo leptos watch

# Release build of the Leptos web app (server binary + wasm bundle)
[working-directory: 'crates/web']
web-release:
    cargo leptos build --release

# Build the Rust JNI library into the Android project's jniLibs
android-lib profile="debug":
    cargo ndk -t arm64-v8a -t armeabi-v7a -o android/app/src/main/jniLibs build -p hyperb-mobile {{ if profile == "release" { "--release" } else { "" } }}

# Build the Android debug APK
android: android-lib
    android/gradlew -p android assembleDebug

# Build and install the Android debug APK on a connected device
android-install: android-lib
    android/gradlew -p android installDebug

# Let a connected device reach the devserver at localhost:9001
android-reverse:
    adb reverse tcp:9001 tcp:9001

# Remove build outputs
clean:
    cargo clean
    rm -rf android/app/build android/app/src/main/jniLibs
