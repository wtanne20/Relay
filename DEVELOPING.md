# Developing Relay

Relay is three pieces that share one app:

| Folder | What it is |
| --- | --- |
| `server/` | The Rust server (`relay.exe`): API, agent, llama-server supervisor. |
| `mobile/` | The app UI (Expo / React Native). The same code runs on the phone and, through react-native-web, in the Windows app. |
| `desktop/` | The Windows app shell (Tauri). It runs the server in the background, shows the `mobile/` UI in a window and sits in the tray. |

Any change under `mobile/src` shows up on both the phone and the desktop app.

## One-time setup (Windows)

1. Install [Node.js 22 LTS](https://nodejs.org), [Rust](https://rustup.rs) (default MSVC toolchain) and the
   [Visual Studio C++ build tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/) ("Desktop development with C++").
2. From the repository folder, run:

   ```powershell
   .\dev\setup.ps1
   ```

   It checks the tools, installs the npm packages, builds the mock model server and adds two firewall rules.
   The rules let the phone reach the dev servers over **Tailscale only** (`100.64.0.0/10`), on ports 8081 (Metro) and 8766 (mock server).
   It asks for administrator rights once, for the firewall.
3. On the phone, install **Relay Dev** from the `Relay-Dev-android` artifact of the
   [Android app workflow](../../actions/workflows/android.yml). It installs next to the normal Relay app.
   You only need a new copy when native modules change, which is rare.

## Everyday loop

```powershell
.\dev\dev.ps1 -Mock
```

- A window opens with **Metro**, the live-reload bundler.
- The **Relay desktop app** starts in dev mode. Edits to `mobile/src` reload instantly. Edits to Rust code in
  `desktop/` or `server/` rebuild and restart the window.
- On the phone, open **Relay Dev** and enter `http://<your PC's Tailscale IP>:8081` (the script prints it), or scan the
  QR code in the Metro window. Edits reload on the phone within a second.

`-Mock` uses a throwaway server with a fake model on port 8766, so you can work on the UI without the GPU. Its data
lives in `dev/data/` (ignored by git). In Relay Dev on the phone, connect to `http://<Tailscale IP>:8766` with the token
from `dev/data/config.toml`.

Without `-Mock`, the desktop app runs the server from source with your real settings (`%LOCALAPPDATA%\Relay`). If the
installed Relay is already running, it connects to that instead. Quit the installed one from its tray icon to
work on server code.

Other switches: `-NoDesktop` starts only Metro (phone work).

On Linux or macOS, `./dev/dev.sh --mock` does the same.

## Checks before pushing

```powershell
cd server;  cargo test; cargo clippy --all-targets
cd mobile;  npx tsc --noEmit
cd desktop\src-tauri; cargo clippy
```

## Release builds

GitHub Actions builds everything on every push:

- **Windows installer** (`windows.yml`): server tests, `relay.exe`, `relay-desktop.exe`, the Inno Setup installer, and a
  silent install/uninstall smoke test.
- **Android** (`android.yml`): `Relay.apk` (normal app) and `Relay-Dev.apk` (dev build).

Tags named `v*` publish a GitHub release with the installer and APK.

To build the desktop app locally: `cd desktop; npx tauri build --no-bundle`. This exports the web UI from `mobile/`
first, and writes `desktop/src-tauri/target/release/relay-desktop.exe`.

## How the desktop app connects

The desktop app reads the token from Relay's `config.toml` and talks to the server at `http://127.0.0.1:<port>`.
Nothing is paired and nothing new is exposed. The server only answers browser requests from the desktop window
(`tauri://localhost`, `http://tauri.localhost`) and the local dev server (`localhost:8081`), and every request still
needs the token. `RELAY_CONFIG` points both the server and the desktop app at a different config file. The dev
scripts use it for `-Mock`.
