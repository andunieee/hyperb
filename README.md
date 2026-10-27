<p align="center">
  <img src="assets/icon.svg" alt="hyperb" width="128">
</p>

<h1 align="center">hyperb</h1>

A browser for [hypereffect](https://crates.io/crates/hypereffect) apps. It opens `hyperb://` addresses served over HTTP, iroh or as wasm modules, see [SPEC.md](SPEC.md) for the protocol.

It comes as a GTK4 desktop app, a terminal app, an Android app and a [web app](https://andunieee.github.io/hyperb/).

# Installation
Grab the files from the [latest release](https://github.com/andunieee/hyperb/releases/latest), or use the commands below, which all download the latest one.

<details>
<summary>Windows</summary>

**Manually**: download the `windows-x64-setup-` `.exe` from the [releases page](https://github.com/andunieee/hyperb/releases) and run it. It adds a Start menu entry and an uninstaller and registers the `hyperb://` and `web+hyperb://` links.

**One-liner** (PowerShell), does the same: downloads the installer and runs it.
```
$u=(irm https://api.github.com/repos/andunieee/hyperb/releases/latest).assets.browser_download_url -match 'windows-x64-setup-v'; iwr $u -OutFile $env:TEMP\hyperb-setup.exe; Start-Process $env:TEMP\hyperb-setup.exe
```

**No installer** (PowerShell): downloads the `.zip`, unzips it to `%LOCALAPPDATA%\hyperb` and you start it with `hyperb.bat`. No Start menu entry and no `hyperb://` links.
```
$u=((irm https://api.github.com/repos/andunieee/hyperb/releases/latest).assets.browser_download_url -match 'windows-x64-v.*\.zip$')[0]; iwr $u -OutFile $env:TEMP\hyperb.zip; Expand-Archive -Force $env:TEMP\hyperb.zip $env:LOCALAPPDATA\hyperb; & $env:LOCALAPPDATA\hyperb\hyperb.bat
```
</details>

<details>
<summary>Linux</summary>

Builds from source (see [Building from source](#building-from-source) for the dependencies). This installs the desktop app for your user, with its icon, and makes it the handler of `hyperb://` and `web+hyperb://` links:
```
git clone https://github.com/andunieee/hyperb && cd hyperb && just desktop-install
```
</details>

<details>
<summary>Android</summary>

Builds from source and installs on every device connected through adb (see [Building from source](#building-from-source)):
```
just android
```
</details>

<details>
<summary>Web</summary>

Nothing to install: open <https://andunieee.github.io/hyperb/>.
</details>

## Releases
Each component is released separately by pushing a tag named `<component>-<N>-<M>` (for example `android-6-1`), which publishes a GitHub release with:
* `desktop-*`: a `.zip` and a `-setup-` installer for Windows
* `android-*`: the APK
* `tui-*`: the terminal app for Linux, macOS and Windows
* `demoserver-*`: the demo server and its wasm apps for Linux, macOS and Windows

The web renderer follows the same tag scheme, but `web-*` tags deploy it to GitHub Pages instead of publishing a release.

## Building from source
### Dependencies
* rustc
* GTK 4.12+ (and its development headers) for the desktop app
* [just](https://github.com/casey/just)
* For Android: `cargo-ndk`, the Android SDK and NDK
* For the web app: `cargo-leptos` and the `wasm32-unknown-unknown` target

### Running
`just` lists the recipes. The main ones:
```
just desktop        # the GTK4 app
just tui            # the terminal app
just demo           # the demo server and the desktop app together
just web            # the web app with hot reload on http://127.0.0.1:3000
```
