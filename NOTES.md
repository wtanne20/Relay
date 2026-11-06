# Relay — Device Host Feature: Working Notes

> Running log for the "phone Proot Linux as a tool host" feature.
> Reference if anything gets lost. **Last updated: 2026-11-05 (pause #2).**

## Core design principle (owner reminder)
**The AI agent (brain) ALWAYS runs on the Windows PC with the 5090 GPU.**
Devices (phone Proot Linux, etc.) are only the *hands*: their filesystem and
shell. Even when a command is issued "on" a device, the thinking, the agent
loop, the session history, and the approvals all stay on the PC. The device
never runs the model and never sees the PC's token or session data.
Do not let any design drift from this.

## Goal
PC (Windows, RTX 5090) = brain. `relay host` on the phone (Proot Linux /
aarch64) = hands. The PC's agent lists/reads/writes/edits/runs in the
device's folder through the existing tool interface, over Tailscale.

## Safety / rollback
- Stable release: **v1.0.3** = commit `acd8aec` (tagged `rollback-v1.0.3`).
- All work on branch **`feature/device-host`** — **NOT committed yet**
  (working tree only). `git checkout acd8aec` = instant rollback.
- Rule: every increment must compile + pass `cargo test` before commit.

## Build environment (IMPORTANT)
- `C:\Users\wtann\.cargo\bin\cargo.exe` / `rustc.exe` are **broken 0-byte
  symlinks** (Windows error: "untrusted mount point"). Do NOT use them.
- Use the real toolchain directly in PATH before each cargo command:
  ```powershell
  $tc='C:\Users\wtann\.rustup\toolchains\stable-x86_64-pc-windows-msvc\bin'
  $env:Path="$tc;$env:Path"; $env:RUSTUP_HOME='C:\Users\wtann\.rustup'; $env:CARGO_HOME='C:\Users\wtann\.cargo'
  Set-Location W:\Projects\AI_Projects\Relay\server
  cargo check   # / cargo test
  ```
  (cargo 1.99.1, rustc 1.99.1, MSVC — verified working from the W: folder.)
- Node 22 works fine for the mobile side.

## STATUS: Increment 1 (device core) — DONE, all tests green, NOT committed

### Verification state (as of pause)
- `cargo check` — clean, zero warnings.
- `cargo test` — **82 passed, 0 failed** (80 pre-existing + 2 new device tests
  that run; the 3rd new test `run_jobs_complete_and_cancel` is in the same
  module — total new: `encodes_urls`, `host_serves_tools_confined_to_root`,
  `run_jobs_complete_and_cancel`).
- **NOT yet done before commit:** `cargo clippy --all-targets` (repo standard
  per DEVELOPING.md), then `git add` + commit.

### Files changed (all on `feature/device-host`, uncommitted)
1. **`server/src/device.rs` (NEW, ~970 lines)** — the whole device feature:
   - `Device` struct + `DeviceRegistry` (list/upsert/get/remove/touch/is_online)
     backed by a new `devices` table in relay.db (additive schema).
   - `host_app(cfg) -> Result<Router>` — axum router for `relay host`:
     `/info`, `/list`, `/read`, `/write`, `/edit`, `/run` (POST→id),
     `/run/{id}` (GET poll, DELETE cancel). All paths confined by the existing
     `tools::Sandbox`; bearer-token auth (constant-time compare); commands run
     as async jobs via `tools::run_shell` (reuses local process plumbing:
     process groups, kill_tree, read_capped).
   - `DeviceClient` — PC-side client: `list_dir`, `read_file`, `write_file`,
     `edit_file`, `run_command` (polls the job, formats output exactly like
     local run_command so the model sees no difference), `cancel_run`.
   - `ensure_schema(conn)` — creates the `devices` table (idempotent).
   - `DeviceError` — mirrors `ApiError`; sandbox denials map to 400 (client
     errors), everything else 500.
   - Unit tests: URL encoding; host serves tools confined to root (401 without
     token, 400 on `../` escape, write+read+edit roundtrip); run job completes.
2. **`server/src/tools/mod.rs`** — `kill_tree` and `read_capped` made
   `pub(crate)` so the device host reuses the exact local process handling.
   Nothing else changed.
3. **`server/src/lib.rs`** — `pub mod device;`; `HostOptions` + `run_host()`
   (binds, prints token, serves until Ctrl+C); `AppInner` gains `devices`
   (built in `build_app` before `AgentManager`).
4. **`server/src/main.rs`** — new `relay host` subcommand: `--bind
   0.0.0.0:8766 --token <t> --name <n> --root <folder> --shell sh --timeout 120`.
   Token is generated once and saved to `host.toml` next to the config so
   restarts keep the same token the PC was paired with.
5. **`server/src/db.rs`** — `open_connection` now calls
   `device::ensure_schema` (additive; old DBs unaffected).
6. **`server/src/api.rs`** — `AppInner.devices`; new routes:
   `GET/POST /devices`, `GET/DELETE /devices/{id}`, `POST /devices/{id}/ping`.
   `add_device` pings the host's `/info` before saving (pairing proves the
   link works); `list_devices` reports `connected` (seen < 2 min).

### Gotchas hit (so we don't re-hit them)
- `Db::call` closures must be `FnOnce + Send + 'static` — clone owned values
  into the closure; use `rusqlite::params!` (not `Params::from_owned(vec![..])`,
  which infers a trait object → E0782).
- `tokio::sync::Mutex` is not `Clone` — share it via `Arc` (see `RunJob`).
- axum header tuple responses need `&str` values; use the
  `([(header::CONTENT_TYPE, mime), (header::CONTENT_DISPOSITION, disp.as_str())], bytes).into_response()`
  pattern (like `get_attachment` in api.rs).
- `HeaderName` is not `Copy` — don't reuse a `header::AUTHORIZATION` binding;
  inline the string `"authorization"` in tests.
- `anyhow_err` helper must be generic `fn anyhow_err<T: Display>(e: T)` to
  accept both `&reqwest::Error` and `&str`.

## NEXT — resume here
1. `cargo clippy --all-targets` in `server/` (fix any lints).
2. Commit increment 1:
   ```powershell
   Set-Location W:\Projects\AI_Projects\Relay
   git add server/src/device.rs server/src/lib.rs server/src/main.rs server/src/db.rs server/src/api.rs server/src/tools/mod.md
   git commit -m "Device hosts: relay host role, device registry, PC-side device client"
   ```
   (double-check the path list — `tools/mod.rs` not `.md`)
3. Then start **Increment 2: routing** (see plan below).

## Increment plan
1. **Device core** ✅ (registry + host app + client + CLI + API + tests)
2. **Routing**: `projects.device_id` column (additive, via `upgrade()` in
   db.rs); agent dispatch: project bound to a device → `DeviceClient`
   methods, else local (unchanged path). Guard + approvals stay PC-side.
   Web tools (web_search/fetch_url) ALWAYS run on the PC. System prompt must
   tell the model the project lives on the device (OS/shell differ, e.g.
   Linux + sh) — `build_system_prompt` in agent/mod.rs ~line 1257 hardcodes
   "run_command uses PowerShell".
   - Seam: agent/mod.rs ~line 577 builds `ToolContext`; ~line 1109-1124 is
     the web-vs-local dispatch where routing plugs in.
3. **Mobile UI**: device list, pair (url+token), bind project to device,
   connected badges. `mobile/src/lib/api.ts` + `types.ts` + a settings screen.
4. **CI + install**: aarch64 Linux cross-compile job, Proot install script
   (static-ish binary + start line).
5. **Stability** (owner requirement): heartbeat/offline state, timeout +
   auto-deny for approvals while device offline, orphan-command cleanup on
   host restart, foreground-service guidance for Android.

## Code map (seams identified)
- `server/src/tools/mod.rs` — `execute(ctx, name, args, cancel)` is the single
  dispatch point for local tools; `run_shell`/`kill_tree`/`read_capped` are
  the process plumbing (now pub(crate)).
- `server/src/agent/mod.rs` ~line 577 — `ToolContext` built per task;
  ~line 1109-1124 — web vs local tool dispatch (where increment-2 routing
  plugs in). `build_system_prompt` ~line 1257.
- `server/src/tools/sandbox.rs` — `Sandbox::new(root, read_only)`, portable.
- `server/src/api.rs` — `router()`, `auth` middleware (bearer, constant-time),
  `ApiError` pattern (mirrored as `DeviceError` in device.rs).
- `server/src/db.rs` — `SCHEMA` is CREATE-IF-NOT-EXISTS; `upgrade()` adds
  columns to old DBs (pattern for `projects.device_id` in increment 2).
- `server/src/config.rs` — `LoadedConfig::load`, XDG/HOME fallbacks already
  there (Linux-ready).
- `server/src/main.rs` — clap `Command` enum (Host added).
- `server/src/lib.rs` — `build_app`/`serve`/`run_host`.
- Tests: `server/tests/integration.rs` Harness pattern (in-process axum +
  mock llama); `hub.rs` tests show the fake-HTTP-server pattern.

## Log
- 2026-11-05 — Branch created, rollback tag set, cargo workaround verified
  (test binary compiled + ran from W:). Seams mapped.
- 2026-11-05 — Wrote device.rs (registry, host app, client, tests), wired
  lib.rs/main.rs/db.rs/api.rs. Fixed 24 of 27 compile errors. **PAUSE #1.**
- 2026-11-05 — Resumed. Fixed last 3 compile errors (params! macro, RunResult
  Clone, anyhow_err generic, Path import in main.rs, HeaderName copy in test).
  Adjusted DeviceError to map sandbox denials → 400. **All 82 tests pass.**
  Remaining: clippy + commit. **PAUSE #2 — resume with clippy + commit,
  then increment 2 (routing).**
