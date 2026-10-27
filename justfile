# List recipes
export ANDROID_HOME := env_var_or_default('ANDROID_HOME', '/opt/android-sdk')
export ANDROID_NDK_HOME := env_var_or_default('ANDROID_NDK_HOME', ANDROID_HOME / 'ndk' / '26.3.11579264')

default:
    @just --list

# Run the GTK4 desktop app
desktop *args:
    cargo run -p hyperb-desktop -- {{args}}

# Install the desktop app for this user (~/.local) as the handler of hyperb:// and web+hyperb:// links
desktop-install:
    #!/usr/bin/env bash
    set -euo pipefail
    cargo build --release -p hyperb-desktop
    target=$(cargo metadata --no-deps --format-version 1 | jq -r .target_directory)
    bin="$HOME/.local/bin/hyperb-desktop"
    apps="${XDG_DATA_HOME:-$HOME/.local/share}/applications"
    install -Dm755 "$target/release/hyperb-desktop" "$bin"
    mkdir -p "$apps"
    icons="${XDG_DATA_HOME:-$HOME/.local/share}/icons/hicolor/scalable/apps"
    install -Dm644 assets/icon.svg "$icons/com.hyperb.app.svg"
    gtk-update-icon-cache -q -t "${XDG_DATA_HOME:-$HOME/.local/share}/icons/hicolor" 2>/dev/null || true
    sed "s#^Exec=hyperb-desktop #Exec=$bin #" crates/desktop/com.hyperb.app.desktop > "$apps/com.hyperb.app.desktop"
    update-desktop-database "$apps" 2>/dev/null || true
    xdg-mime default com.hyperb.app.desktop x-scheme-handler/hyperb x-scheme-handler/web+hyperb
    echo "installed $bin; hyperb:// and web+hyperb:// links now open in it"

# Run the terminal app
tui *args:
    cargo run -p hyperb-tui -- {{args}}

wasm_apps := "qr textstats billsplit finance chart minecraft dbclient"

# Build the stateless demo apps as wasm modules into wasm-apps/
wasm-apps:
    #!/usr/bin/env bash
    set -euo pipefail
    mkdir -p wasm-apps
    target=$(cargo metadata --no-deps --format-version 1 | jq -r .target_directory)
    for app in {{wasm_apps}}; do
        cargo build -q --target wasm32-unknown-unknown --release -p hyperb-wasmapps --features "$app"
        cp "$target/wasm32-unknown-unknown/release/wasmapps.wasm" "wasm-apps/$app.wasm"
    done

# Serve the demo apps over HTTP + SSE (default 127.0.0.1:9001), iroh and wasm, and print their hyperb:// addresses
demoserver addr="127.0.0.1:9001": wasm-apps
    cargo run -p hyperb-demoserver -- {{addr}}

# Run the demoserver and the desktop app together
demo:
    #!/usr/bin/env bash
    set -euo pipefail
    just wasm-apps
    cargo build -p hyperb-demoserver -p hyperb-desktop
    cargo run -q -p hyperb-demoserver &
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

# Build the dev build and install it on all devices connected through adb
android:
    cargo ndk -t arm64-v8a -t armeabi-v7a -o android/app/src/main/jniLibs build -p hyperb-mobile
    android/gradlew -p android installDebug

# Remove build outputs
clean:
    cargo clean
    rm -rf android/app/build android/app/src/main/jniLibs
