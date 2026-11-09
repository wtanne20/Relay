# Relay — Device Host + File Download + PWA + Linux: Working Notes

> Running log. **Last updated: 2026-11-09 — linked computers (one app, code pairing, outgoing link).**

## Linked computers (2026-11-09, branch `claude/project-thread-cz7oc3`)

Goal (owner's words): install the app on e.g. a Linux x86-64 laptop; the AI reads and changes that
laptop's local files while the main desktop is the brain. Easy to install and connect, for any user.
Always over Tailscale. Clear indication of which computer is the brain, and which computer a project's
or chat's files are on. x86 Linux desktop first; aarch64 Proot on the phone next.

What changed:
- **The link goes the other way.** A linked computer dials *out* to the brain (`GET /devices/link`,
  WebSocket) and answers file/command requests with the same host router `relay host` serves. No port
  opened on the laptop, no IP guessing, survives the brain restarting (reconnects with backoff).
  `server/src/link.rs` (brain hub + device loop + pairing + Tailscale discovery).
- **Pairing with a 6-digit code.** Brain: Settings > Computers > Link a computer shows a code (10 min,
  single use, 5 tries). The computer trades it for its own revocable key (`POST /devices/claim`).
- **Projects have a folder on a computer.** `POST /projects {device_id, path}`; the device confines each
  request to that folder (`x-relay-folder`), inside what it shares. Browse folders from the brain.
  Downloads from device projects work too. Every project carries `location`.
- **One desktop app.** First launch asks "This computer is the brain" or "Use another computer as the
  brain" (finds brains on the tailnet via `/hello`, takes the code, picks the shared folder). A linked
  computer starts on login (XDG autostart) and lives in the tray. Existing installs stay brains.
- **Indicators.** Brain card at the top of the sidebar (and on the phone home + Computers screen):
  which computer is the brain, and this computer's role/link state. Project rows, the project screen and
  the chat header show which computer the files are on, with an online dot.
- CLI for headless/Proot: `relay link --brain <addr> --code <code> [--root DIR]`, then `relay link`;
  `relay unlink`. Without --brain it lists brains it finds.
- Removed: the separate Relay Client app (`client/`, `dev/relay-client/`) and `relay pair-device`.

Verified: 88 unit + 13 integration + 6 supervisor tests (new end-to-end test: pair with a code, link,
create a project on the device, the brain's agent writes a file there, download it, go offline, revoke);
clippy clean (server + desktop on Linux); `tsc` clean; a real brain + `relay link` on one box; UI
screenshots of setup, sidebar, Computers, pairing and new-project-on-device.
NOT verified: the Tauri window on a real Linux desktop or Windows; two machines over real Tailscale.

## v1.0.6 — RELEASED (2026-11-06) — COMPLETE (6 assets)

Four features shipped:
1. **File downloads** — agent offers a link, app downloads + opens it.
2. **PWA for aarch64** — brain serves the web UI to any browser.
3. **Linux x64 desktop** — full brain + native window (.deb + .AppImage).
4. **Load model on demand** (from the v1.0.4 line, carried forward).

### v1.0.6 release assets (all published)
- `RelaySetup-1.0.6.exe` (13.4 MB) — Windows brain + desktop UI + web bundle
- `Relay.apk` (110 MB) — Android app
- `Relay-Dev.apk` (259 MB) — Android dev build
- `relay-host-aarch64-linux.zip` (6.1 MB) — aarch64 "hands" + `pwa` launcher
- **`Relay-1.0.6-amd64.deb` (10.2 MB) — Linux x64 desktop (Debian/Ubuntu)**
- **`Relay-1.0.6-x86_64.AppImage` (84.7 MB) — Linux x64 desktop (portable)**
- **`relay-host-x86_64-linux.zip` (6.4 MB) — x86_64 "hands" (link a Linux PC to the brain)**
- **`RelayClient-1.0.6-amd64.deb` (10.7 MB) — GUI "install and go" client (Claude-like)**
- **`RelayClient-1.0.6-x86_64.AppImage` (85 MB) — GUI client (portable)**

### Linking a full Linux PC to the Windows brain (device-host, x86_64)
- The Linux PC runs `relay host` (hands); the Windows PC keeps the brain.
- `install.sh` now auto-detects arch (x86_64 vs aarch64) and pulls the right zip.
- Flow: `./install.sh` → `~/.relay-host/start` (prints address+token) →
  Windows app: Settings → Devices → Pair → bind a project to the device.
- The AI then reads/edits/runs on the Linux PC's files, powered by the Windows model.

### Relay Client (the "install and just works" app — Claude-like)
Goal: install on a laptop, and it uses the local files while the home desktop
does the processing. (User's exact ask, 2026-11-08.)

**V2 — a real GUI app (no terminal)** (2026-11-09, replaces the terminal .deb):
- **`RelayClient-<v>-amd64.deb`** (10.7 MB) + **`RelayClient-<v>-x86_64.AppImage`**
  (85 MB) — a small Tauri window (dark/green, matching Relay).
- Double-click "Relay Client" → enter the brain's address + token once →
  **Connect**. It then: tests the brain, starts this machine's host
  (exposing its files + shell), pairs it with the brain, and offers "Open Relay"
  (the UI). Remembers the brain, so later launches show the connected state.
- The relay binary is **bundled inside the app** (Tauri resource), so the .deb
  is self-contained — no separate binary install.
- Files: `client/src-tauri/` (Rust backend + Tauri config),
  `client/web/index.html` (the setup screen), `client/package.json` + lock.
- `server/src/main.rs` — new `relay pair-device` command (the client uses it).
- CI: Linux workflow builds it (bundles the relay binary as a resource).
- Verified: cargo check + clippy clean (Windows baseline); Linux CI build
  succeeded; valid .deb + AppImage, downloads cleanly.
- NOT verified: real install + connect on a Linux laptop (user to test).

**V1 — terminal-based (superseded):** `dev/relay-client/` launcher + .deb.
Still in the repo, but the GUI app (V2) is the one to use.

### Rollback
- `rollback-v1.0.6` → `3118398` (file-download done, PWA not started)
- `v1.0.6` → `8616047` (release)

## Linux x64 desktop (NEW in v1.0.6)

Goal: a full Linux x64 release — the brain + native window, exactly like
Windows (unlike aarch64, which is Proot/hands-only + PWA).

### How it works
- Tauri can't cross-compile from Windows, so the Linux build runs on a
  **Linux CI runner** (`.github/workflows/linux.yml`).
- The desktop code was already cross-platform:
  - `config.rs` data dir: `~/.local/share/relay` on Linux (not just %LOCALAPPDATA%)
  - `setup.rs::open_in_browser`: uses `xdg-open` on Linux
  - `config.rs::LLAMA_SERVER_EXE`: `llama-server` on Linux, `.exe` on Windows
- Tauri bundling enabled in `tauri.conf.json` (`deb`, `appimage`); the Windows
  build uses `--no-bundle` so it's unaffected.

### DONE (all verified)
- [x] `.github/workflows/linux.yml` — Linux CI (deps, test, Tauri build,
      rename, upload, publish-on-tag)
- [x] `tauri.conf.json` — `bundle.active: true`, targets `deb`, `appimage`
- [x] CI build **succeeded** on a Linux runner (tests passed on Linux too)
- [x] Artifacts: `Relay-1.0.6-amd64.deb` + `Relay-1.0.6-x86_64.AppImage`
- [x] Both attached to the v1.0.6 release

### Tauri Linux build dependencies (installed by CI)
libwebkit2gtk-4.1-dev, libgtk-3-dev, libayatana-appindicator3-dev,
librsvg2-dev, libssl-dev, libfuse2, patchelf, file, build-essential, curl,
pkg-config

### NOT VERIFIED (needs a real Linux x64 box)
- [ ] Install the .deb or run the .AppImage on a real x64 Linux desktop
- [ ] Verify the brain runs (model loads, agent works) in the native window
- [ ] Verify the PWA also works from a browser on that box (same brain)

## PWA for aarch64 Linux (v1.0.6)

### How the aarch64 box works (owner's requirement met)
- **Brain stays on the Windows PC** (model, agent, history, approvals).
- aarch64 box runs `relay host` (exposes its files + shell) — the AI reads/edits
  Proot files via the paired device. (Already in v1.0.5, unchanged.)
- aarch64 box opens the brain's URL in a browser and installs the PWA for the
  UI. No brain on the aarch64 box.

### PWA implementation (the new work)
- `server/src/ui.rs` — middleware serves the web UI to browser navigations
  (Accept: text/html) + static assets; API calls (Accept: application/json)
  pass through to the authed API untouched. Same origin, so no CORS.
- PWA manifest served at `/manifest.webmanifest` + injected into the served
  index.html.
- `web_root()` finds the bundle next to the exe, in `web/`, or `mobile/dist`.
- Windows installer copies `mobile/dist` → `<app>/web`; CI builds it.
- aarch64 host package includes a `pwa` launcher (opens the brain's UI).
- Connect screen pre-fills `window.location.origin` on web (the brain's address).

### VERIFIED (live tests)
- UI: `/` + `/devices` serve the SPA, 1.1MB bundle loads, manifest 200.
- API: `/info`, `/projects` work with token (200 JSON), 401 without.
- **Installed layout**: relay.exe + web/ (run from a different cwd) serves the
  UI + bundle + manifest — the CI smoke-test gap, verified locally.
- 83 unit + 12 integration + 6 supervisor tests pass; clippy clean; mobile tsc clean.

### Commits (v1.0.6, newest first)
- `8616047` — Bump to v1.0.6
- `bbed0d5` — PWA: pre-fill the brain address on the web connect screen
- `5ba03c9` — aarch64: add a PWA launcher to the host package
- `18a6a36` — Ship the web UI with the Windows brain (installer + CI)
- `e2672b0` — PWA: serve a manifest + inject it into the app HTML
- `a36aed4` — Serve the web UI from the brain (for the aarch64 PWA)
- `3118398` — File downloads

### NOT VERIFIED (needs a real aarch64 box)
- [ ] Open the brain's URL in a browser on the Proot box, install the PWA,
      verify chat + file download work.
- [ ] Run `relay host` + pair + have the AI edit a Proot file end-to-end.
- [ ] (Optional) a nicer PWA icon (currently uses favicon.ico).

## PWA for aarch64 Linux (v1.0.6) — CORE DONE

Goal: the aarch64 Proot box gets a UI that looks/works like the Windows desktop
app, but without the brain. The AI reads/edits Proot files via `relay host`
(already built in v1.0.5). The PWA is just the control panel.

### Approach (decided)
- The Windows app is a Tauri window + the same web UI bundle (Expo web export).
- Proot can't run a native webkit/GTK app, so the **brain serves the same web
  bundle** to any browser. The aarch64 box opens it and installs it as a PWA.
- Same appearance, same features (it's literally the same code). The aarch64
  box needs NO UI binary — just a browser + `relay host` (already in v1.0.5).

### DONE (all verified)
- [x] `server/src/ui.rs` — middleware serves the web UI to browser navigations
  (Accept: text/html) + static assets; API calls (Accept: application/json)
  pass through to the authed API untouched. Same origin, so no CORS.
  - Resolves the root-absolute-path blocker: UI is served at the root (where
    the bundle expects `/_expo/...`), not under a prefix.
  - `web_root()` finds the bundle next to the exe, in `web/`, or `mobile/dist`.
- [x] PWA manifest served at `/manifest.webmanifest` + injected into the
  served index.html (name, theme, standalone display).
- [x] Windows installer copies `mobile/dist` → `<app>/web`; CI builds it.
- [x] aarch64 host package now includes a `pwa` launcher (opens the brain's
  UI in a browser for "add to home screen"); `dev/pwa.sh` + install.sh.
- [x] Wired into `lib.rs` + `api.rs` (middleware, outer than auth).

### VERIFIED (live, throwaway server)
- `GET /` (Accept: text/html) → 200 HTML ✓
- `GET /devices` (browser nav) → 200 SPA index.html ✓
- `GET /_expo/.../entry-*.js` → 200, 1.1MB bundle ✓
- `GET /info` WITH token (Accept: application/json) → 200 JSON ✓
- `GET /info` WITHOUT token → 401 (not the UI) ✓
- `GET /projects` WITH token → 200 JSON (the collision case) ✓
- `GET /manifest.webmanifest` → 200 valid JSON ✓
- HTML has manifest link + theme-color ✓
- 83 unit + 12 integration + 6 supervisor tests pass; clippy clean

### Commits (newest first)
- `18a6a36` — Ship the web UI with the Windows brain (installer + CI)
- `e2672b0` — PWA: serve a manifest + inject it into the app HTML
- `a36aed4` — Serve the web UI from the brain (for the aarch64 PWA)
- `3118398` — File downloads (previous feature)

### NOT STARTED (to ship as v1.0.6)
- [ ] Bump version to 1.0.6 (server, mobile, desktop, installer, app.json)
- [ ] Push + tag v1.0.6 → CI builds Windows installer (with web/ bundle) +
      APKs + aarch64 host (with pwa launcher)
- [ ] Real-device test: open the brain's URL in a browser on the aarch64 box,
      install the PWA, verify it works; run `relay host` + pair + edit files
- [ ] (Optional) a nicer PWA icon (currently uses favicon.ico)

## PREVIOUS WORK: File download from the app (DONE, committed as 3118398)

### DONE (all verified)
- [x] Server endpoint `GET /projects/{id}/download?path=...` (api.rs)
  - Sandbox-validated (can't escape the project folder)
  - Rejects device-bound projects (files are on the remote machine)
  - Sets `Content-Disposition: attachment` + correct MIME type
  - Compiles clean
- [x] `DownloadQuery` struct + `file_mime` helper
- [x] Agent prompt hint (`DOWNLOAD_HINT` in agent/mod.rs): tells the model to
  present deliverables as `[Download <name>](relay-download://<path>)`
- [x] Mobile: `mobile/src/lib/download.tsx` — `downloadAndOpen` (Android:
  expo-file-system downloadAsync → intent open; web/desktop: blob link) +
  `DownloadProvider`/`useDownload` context
- [x] Mobile: `Markdown.tsx` — `DownloadButton` renders `relay-download://`
  links as a download button (busy spinner, error text)
- [x] Mobile: `session/[id].tsx` — chat wrapped in `DownloadProvider`

### VERIFIED (live end-to-end test, throwaway server)
- Auth: 401 without token ✓
- Download: HTTP 200, correct content, `Content-Disposition: attachment`,
  correct MIME (text/plain, application/vnd.android.package-archive) ✓
- Path traversal `../../secret`: 400 "access denied ... outside the project" ✓
- Missing file: 400 ✓
- 83 unit + 12 integration + 6 supervisor tests pass; clippy clean; mobile tsc clean

## STATUS: COMPLETE (increments 1-5 committed, all tests green, clippy clean)

Branch `feature/device-host` commits (newest first):
- `bb43287` — 5. Stability: no-hang when a device drops (connect timeout, poll grace, fail-fast at session start, test).
- `e9d3a81` — 4. CI (aarch64 host.yml) + Proot install script.
- `71bc04a` — notes.
- `01c6800` — 3. Mobile: pair devices + bind a project to one.
- `30387da` — 2. Routing: project's tools run on a paired device.
- `7faebc4` — 1. Device core: relay host role, registry, client, CLI, API.
- `acd8aec` — v1.0.3 stable (rollback tag `rollback-v1.0.3`).

Rollback to stable: `git checkout acd8aec`. To a specific increment: check out that commit.

## Core design principle (owner reminder)
**The AI agent (brain) ALWAYS runs on the Windows PC with the 5090 GPU.**
Devices (phone Proot Linux, etc.) are only the *hands*: their filesystem and
shell. Even when a command is issued "on" a device, the thinking, the agent
loop, the session history, and the approvals all stay on the PC. The device
never runs the model and never sees the PC's token or session data.

## What the feature does (final)
PC (Windows, 5090) = brain. `relay host` on a phone (Proot Linux / aarch64) =
hands. A project can be bound to a device; then its 5 file/shell tools
(list_dir/read_file/write_file/edit_file/run_command) run on that machine while
the model, session history and approvals all stay on the PC. Web tools always
run on the PC. The PC guard still applies to device commands.

## Build environment (IMPORTANT)
- `C:\Users\wtann\.cargo\bin\cargo.exe` / `rustc.exe` are **broken 0-byte
  symlinks** (Windows "untrusted mount point"). Use the real toolchain in PATH
  before each cargo command:
  ```powershell
  $tc='C:\Users\wtann\.rustup\toolchains\stable-x86_64-pc-windows-msvc\bin'
  $env:Path="$tc;$env:Path"; $env:RUSTUP_HOME='C:\Users\wtann\.rustup'; $env:CARGO_HOME='C:\Users\wtann\.cargo'
  Set-Location W:\Projects\AI_Projects\Relay\server
  cargo test   # 83 unit + 11 integration + 5 supervisor, all green
  cargo clippy --all-targets   # clean
  ```
  (cargo/rustc 1.99.1, MSVC. Clippy was missing; added via `rustup component add clippy`.)
- Mobile: `cd mobile; npm install; npx tsc --noEmit` (clean); prettier clean;
  `npx expo export --platform web` bundles (devices.js present).

## Files (by increment)
1. `server/src/device.rs` (NEW ~1000 lines): Device/DeviceRegistry, host_app
   (list/read/write/edit/run jobs), DeviceClient, ensure_schema, tests.
   `server/src/lib.rs` (HostOptions/run_host/AppInner.devices),
   `server/src/main.rs` (relay host CLI), `server/src/db.rs` (devices table),
   `server/src/api.rs` (/devices routes), `server/src/tools/mod.rs`
   (kill_tree/read_capped pub(crate)).
2. `server/src/db.rs` (projects.device_id), `server/src/api.rs`
   (PATCH device_id + validate_device), `server/src/agent/mod.rs`
   (run_tool/run_on_device + # Device prompt), `server/src/tools/guard.rs`
   (check_command_remote), `server/src/lib.rs`, `server/tests/integration.rs`.
3. `mobile/src/app/devices.tsx` (NEW), `mobile/src/app/project/[id].tsx`
   (Runs-on chip + DeviceSheet), `mobile/src/app/settings.tsx` (Devices row),
   `mobile/src/lib/api.ts` (device methods), `mobile/src/lib/types.ts` (Device).
4. `.github/workflows/host.yml` (NEW), `dev/proot-install.sh` (NEW, +x).
5. `server/src/device.rs` (connect timeout + poll grace + unreachable test),
   `server/src/agent/mod.rs` (fail-fast at session start).

## How to use it (end-to-end, once a release artifact exists)
1. Phone (Proot Linux): `./install.sh` (downloads the aarch64 zip), then
   `~/.relay-host/start`. It prints the address + token.
2. PC: Relay app → Settings → Devices → Pair a device → paste address + token.
3. PC: open a project → "Runs on" chip → pick the device.
4. Chat as normal; the agent's file/shell tools run on the phone, the model
   stays on the PC.

## Gotchas hit (so we don't re-hit them)
- `Db::call` closures must be `FnOnce + Send + 'static` — clone owned values;
  use `rusqlite::params!` (not `Params::from_owned(vec![..])` → E0782).
- `tokio::sync::Mutex` is not `Clone` — share via `Arc` (see `RunJob`).
- axum header tuple responses need `&str` values (like `get_attachment`).
- `HeaderName` is not `Copy` — inline `"authorization"` in tests.
- `anyhow_err<T: Display>(e: T)` generic to accept `&reqwest::Error` and `&str`.
- `Project` struct gained `device_id` — any direct `Project {..}` initializer
  must include it (integration test).
- A device-bound project's `path` lives on the device, not the PC — don't
  require it to exist locally.
- reqwest "connection refused" surfaces as "error sending request for url ..."
  (no "refused"/"connect" substring) — assert on "error" + timing, not wording.
- `let-chains` (Rust 2024) accepted by clippy; use to avoid collapsible-if.

## STATUS: v1.0.5 RELEASE COMPLETE (2026-11-06)

The device-host feature is released as **v1.0.5** on GitHub, with all assets
published (each has the SHA-256 the app's updater requires):

- **`RelaySetup-1.0.5.exe`** (12.5 MB) — Windows brain + desktop UI (auto-updatable)
- **`Relay.apk`** (110 MB) — Android app
- **`Relay-Dev.apk`** (259 MB) — Android dev build
- **`relay-host-aarch64-linux.zip`** (6.1 MB) — the "hands" binary for the Proot aarch64 desktop

How the app finds them: the updater (`server/src/updates.rs`) checks the
**latest** GitHub release for these exact names; it needs a SHA-256 to run the
Windows installer. Bumping the version is what makes the update appear. CI
publishes them on any `v*` tag (windows.yml, android.yml, host.yml).

### Release timeline (v1.0.5)
- `v1.0.4` already existed on GitHub (a separate "load model on demand" line).
  So device-host was merged **on top of it** and released as v1.0.5.
- Merged `feature/device-host-v2` (device-host on latest main) → main.
- Bumped all version files (server, mobile, desktop, installer, app.json) to 1.0.5.
- **Fix 1 (Windows CI fail):** `server/Cargo.lock` + `desktop Cargo.lock` were
  still at 1.0.4 → broke the `--locked` build. Synced to 1.0.5.
- **Fix 2 (aarch64 CI fail):** bare cross-gcc lacked libc headers for `ring`
  (rustls). Added `libc6-dev-arm64-cross` to `host.yml`. (A `cross` attempt
  failed on archive layout; the apt cross-gcc + headers approach works.)
- Moved the `v1.0.5` tag to the fixed commit (8e10eef) and re-triggered CI.
- Attached the aarch64 host zip (built on main with the fix) to the release.

### Rollback
- `rollback-v1.0.3` → acd8aec (stable v1.0.3).
- v1.0.4 tag → the load-on-demand release (pre-device-host).
- v1.0.5 tag → 8e10eef (device-host + load-on-demand).
- Local branches: `main`, `feature/device-host-v2` (release), `feature/device-host` (original).

## Multi-platform roadmap (owner requirement, 2026-11-05)

The brain (`relay serve`), hands (`relay host`), and UI (app) are already
separable and platform-agnostic. "Set a device as host" = run `relay serve`
on it + point the app at it. No code change needed for that.

| Platform  | Brain       | Hands       | UI (app)   | Status                          |
|-----------|-------------|-------------|------------|---------------------------------|
| Windows   | relay serve | relay host  | Tauri      | All exist. Needs release build. |
| Android   | Possible    | relay host  | Phone app  | App + hands exist.              |
| Linux x64 | relay serve | relay host  | Tauri      | Code cross-platform. Needs CI.  |
| Linux aarch | Possible  | relay host  | Phone app  | Hands built by CI. Brain CPU.   |

**Priority (owner):** Windows + Android first, then Linux one at a time.

**Future edits needed:**
1. Linux x64 CI job (server + Tauri desktop target).
2. Linux aarch64 brain test (CPU-only, slow).
3. Tauri Linux build (currently Windows-only in CI).
4. Release pipeline: add Linux x64 server + desktop to the v* tag workflow.
5. Shared device registry (so hands survive a brain switch).
6. `relay host` should detect its Tailscale IP and print the exact address
   to paste (currently prints 0.0.0.0:8766; user must type the IP).

## Suggested next steps (not started; optional)
- End-to-end: run `relay host` in a Linux VM/Proot, pair from the app, bind a
  project, and drive a real agent session against it.
- README section documenting the device-host feature + the aarch64 artifact.
- Per-device "connected" heartbeat in the agent loop (currently the connected
  dot is computed from last_seen < 2 min).
- Bump version + tag `v1.0.4` when ready to release (CI publishes the host zip
  and the Windows installer on v* tags).

## Log
- 2026-11-05 — Branch + rollback tag; cargo workaround verified; seams mapped.
- 2026-11-05 — device.rs written + wired; 24/27 errors fixed. PAUSE #1.
- 2026-11-05 — last 3 errors fixed; DeviceError sandbox→400; 82 tests pass. PAUSE #2.
- 2026-11-05 — clippy (installed) + lint fixed; committed increment 1 (7faebc4).
- 2026-11-05 — increment 2 (routing) done; committed (30387da).
- 2026-11-05 — increment 3 (mobile UI) done; tsc/prettier/web-export clean; committed (01c6800).
- 2026-11-05 — increment 4 (CI + Proot install) committed (e9d3a81).
- 2026-11-05 — increment 5 (stability: no-hang) + test; 83 tests green, clippy clean; committed (bb43287). **Feature complete.**
