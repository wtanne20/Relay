# Relay — Device Host Feature: Working Notes

> Running log for the "phone Proot Linux as a tool host" feature.
> Reference if anything gets lost. **Last updated: 2026-11-05 — ALL 5 INCREMENTS DONE.**

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
