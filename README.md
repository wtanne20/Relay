# Relay

Relay is a personal coding agent you control from your phone.

- **`server/`**: a Rust server (binary `relay`) that runs on your Windows PC. It starts and supervises
  llama.cpp's `llama-server` with your local model, runs an agent loop with file and PowerShell tools
  restricted to each project folder, stores everything in SQLite, and exposes a token-protected REST +
  WebSocket API.
- **`mobile/`**: an Expo (React Native) app for iOS and Android. You manage projects and sessions, send
  prompts, watch the agent stream its work, stop it, and get a push notification when a task ends.

Your phone reaches the PC over **Tailscale only**. Nothing is exposed to the public internet:
llama-server listens on `127.0.0.1`, and Relay should only be reachable through your tailnet.

```
 Phone (Expo app) ──Tailscale──▶ Relay :8765 (REST + WebSocket, bearer token)
                                   │
                                   ├─ agent loop ─▶ llama-server 127.0.0.1:8080 (/v1/chat/completions, streaming)
                                   ├─ tools: list_dir, read_file, write_file, edit_file, run_command (PowerShell)
                                   ├─ SQLite: projects, sessions, messages, push tokens
                                   └─ Expo Push API (project name, session title, status only)
```

The full API is documented in [API.md](API.md).

## Contents

1. [Install Rust](#1-install-rust)
2. [Install llama.cpp](#2-install-llamacpp)
3. [Build Relay](#3-build-relay)
4. [Migrate from LM Studio Bionic](#4-migrate-from-lm-studio-bionic-relay-migrate)
5. [Start the server](#5-start-the-server)
6. [Run at Windows startup](#6-run-relay-at-windows-startup)
7. [Tailscale on the PC and phone](#7-tailscale-on-the-pc-and-phone)
8. [The mobile app with Expo Go](#8-the-mobile-app-with-expo-go-development)
9. [Installable apps with EAS Build](#9-installable-apps-with-eas-build)
10. [Push notifications](#10-push-notifications)
11. [Configuration reference](#configuration-reference)
12. [Instructions, AGENTS.md and skills](#instructions-agentsmd-and-skills)
13. [Testing without a GPU](#testing-without-a-gpu)
14. [Security notes](#security-notes) and [known limitations](#known-limitations)

---

## 1. Install Rust

1. Install **Visual Studio Build Tools** (the MSVC linker and Windows SDK Rust needs):
   - Download "Build Tools for Visual Studio" from <https://visualstudio.microsoft.com/downloads/>
     (under *Tools for Visual Studio*).
   - In the installer, tick the **Desktop development with C++** workload. Make sure *MSVC … C++ x64/x86 build
     tools* and a *Windows 11 SDK* (or 10) are selected. Install.
2. Install Rust with **rustup** from <https://rustup.rs> (run `rustup-init.exe`, accept the defaults,
   which select the `x86_64-pc-windows-msvc` toolchain).
3. Open a new terminal and check:

   ```powershell
   rustc --version
   cargo --version
   ```

## 2. Install llama.cpp

1. Open <https://github.com/ggml-org/llama.cpp/releases> and download, from the latest release:
   - `llama-<build>-bin-win-cuda-<version>-x64.zip` (the CUDA build), and
   - `cudart-llama-bin-win-cuda-<version>-x64.zip` (the CUDA runtime DLLs), with the **same CUDA version**.

   Pick the CUDA version your NVIDIA driver supports (`nvidia-smi` shows the highest supported CUDA version
   top right). You do not need the full CUDA Toolkit.
2. Extract **both** zips into the same folder, for example `C:\llama.cpp`. It should contain
   `llama-server.exe` next to `ggml-cuda.dll`, `cudart64_*.dll` and `cublas64_*.dll`.
3. Optional check:

   ```powershell
   C:\llama.cpp\llama-server.exe --version
   ```

Relay launches llama-server itself with `-m <model_path> --port <port>` plus these defaults (editable in
`config.toml`):

```
--jinja -ngl 99 -c 65536 --flash-attn on --cache-type-k q8_0 --cache-type-v q8_0
--temp 0.6 --top-k 20 --top-p 0.95 --host 127.0.0.1
```

`--jinja` is required for tool calling. Relay always forces `--host 127.0.0.1`.

## 3. Build Relay

```powershell
git clone https://github.com/wtanne20/Relay.git
cd Relay\server
cargo build --release
```

The binary is `server\target\release\relay.exe`. Relay keeps its data next to `config.toml`
(by default the folder you run it from, so `server\`):

| File | Purpose |
|---|---|
| `config.toml` | settings and the API token (created on first run) |
| `relay.db` | projects, sessions, messages, push tokens |
| `logs\llama-server.log` | llama-server output |
| `system_prompt.md` | global system prompt (created on first run) |
| `skills\` | your SKILL.md folders |

Run the tests any time with `cargo test` (they use a mock llama-server, no GPU needed).

## 4. Migrate from LM Studio Bionic (`relay migrate`)

`relay migrate` imports your setup without deleting, moving or modifying any original files.
Bionic chat history is not migrated.

```powershell
cd Relay\server

# Import specific project folders, point Relay at llama.cpp, use your Bionic system prompt,
# and pick the model from the GGUF files under %USERPROFILE%\.lmstudio\models:
.\target\release\relay.exe migrate C:\code\app1 C:\code\app2 `
    --llama-server C:\llama.cpp `
    --system-prompt C:\Users\me\Documents\bionic-prompt.txt

# Or scan a parent folder and confirm each subfolder (y/N):
.\target\release\relay.exe migrate --scan C:\code
```

What it does:

1. **Projects**: each folder becomes a project named after the folder (already-imported folders are skipped).
2. **Skills**: every folder containing `SKILL.md` under `%USERPROFILE%\.lmstudio\skills` (and any
   `--skills <path>`) is **referenced** in `skills_paths` in config.toml. Nothing is copied.
3. **System prompt**: with `--system-prompt <file>`, the text becomes `system_prompt.md`
   (an existing one is backed up to `system_prompt.md.bak`).
4. **Model**: lists the GGUF files under `%USERPROFILE%\.lmstudio\models` with sizes and asks which one to use
   (press Enter to keep the current choice). The file stays where LM Studio keeps it.
5. Prints a summary.

Other options: `--lmstudio <dir>` if LM Studio's folder is elsewhere, `--yes` to accept every scanned
folder, `--skip-model` to skip the model picker. `relay migrate --help` lists everything.

## 5. Start the server

```powershell
cd Relay\server
.\target\release\relay.exe          # same as: relay.exe serve
```

On first run Relay creates `config.toml`, generates a random API token and prints it:

```
  Relay 0.1.0 listening on 0.0.0.0:8765
  API token: 3f9c…e1a7
  Connect from your phone over Tailscale: http://<this PC's Tailscale name or IP>:8765
```

Then it starts llama-server (or reuses one already answering on the port), waits for `/health`, and
reports the model as ready. Loading a 27B model takes a little while; the app shows "Model loading…".

- If llama-server exits unexpectedly, Relay restarts it (at most 3 times in 5 minutes, then it reports an error
  until you press **Restart model** in the app or call `POST /model/restart`).
- **Ctrl+C** (or closing the window) stops running tasks and llama-server cleanly.

When Windows Firewall asks, allow Relay on **private** networks only. Better, allow port 8765 only from
Tailscale addresses (run PowerShell as administrator):

```powershell
New-NetFirewallRule -DisplayName "Relay (Tailscale only)" -Direction Inbound -Protocol TCP `
  -LocalPort 8765 -RemoteAddress 100.64.0.0/10 -Action Allow
```

Quick check from the PC:

```powershell
$t = (Select-String -Path config.toml -Pattern '^token = "(.*)"').Matches[0].Groups[1].Value
Invoke-RestMethod http://127.0.0.1:8765/info -Headers @{ Authorization = "Bearer $t" }
```

There is also a small command-line client that sends a prompt and prints the streamed events:

```powershell
cargo run --release --example relay_cli -- --project C:\code\app1 "list the files and summarize the project"
```

## 6. Run Relay at Windows startup

`server\start_relay.bat` starts the release build with the config in `server\`:

```bat
cd /d "%~dp0"
"target\release\relay.exe" serve --config "%~dp0config.toml"
```

To start it automatically when you log in, either:

- **Startup folder** (simplest): press `Win+R`, type `shell:startup`, and put a shortcut to `start_relay.bat`
  there. Set the shortcut to *Run: Minimized* if you like.
- **Task Scheduler** (starts even if you forget to open anything): *Create Task…* → Triggers: *At log on* (your
  user) → Actions: *Start a program* → `C:\path\to\Relay\server\start_relay.bat`, *Start in*:
  `C:\path\to\Relay\server` → Conditions: untick *Start the task only if the computer is on AC power*.

The PC must stay awake while you use Relay from your phone (Settings → System → Power → Sleep: Never, or use
a tool such as PowerToys Awake).

## 7. Tailscale on the PC and phone

1. Install Tailscale on the PC (<https://tailscale.com/download/windows>) and sign in.
2. Install the Tailscale app on your phone (App Store / Play Store) and sign in with the **same account**.
3. In the Tailscale admin console (<https://login.tailscale.com/admin/machines>) find the PC's address:
   a `100.x.y.z` IP, or a MagicDNS name like `my-pc.tail1234.ts.net` (enable MagicDNS under *DNS*).
4. In the app's Connect screen, enter `my-pc.tail1234.ts.net` (port 8765 is added automatically) or
   `100.x.y.z:8765`, paste the token, tap **Test connection**, then **Save**.

Do not use Tailscale Funnel or router port forwarding for Relay; the API should never be public.

Traffic between the phone and PC is plain HTTP inside Tailscale's encrypted WireGuard tunnel. The app
therefore allows cleartext HTTP (Android `usesCleartextTraffic`, iOS ATS exception in `app.json`).

## 8. The mobile app with Expo Go (development)

Requirements: Node.js LTS on the PC and the **Expo Go** app on your phone (it must support Expo SDK 58).

```powershell
cd Relay\mobile
npm install
npx expo start
```

Scan the QR code with the camera (iOS) or Expo Go (Android). The phone needs to reach the dev server: with
both on the same Wi-Fi it just works; over Tailscale only, use `npx expo start --tunnel` or set
`REACT_NATIVE_PACKAGER_HOSTNAME=<PC's Tailscale IP>` before `npx expo start`.

Screens:

- **Connect**: server address + token, test, save (stored in the secure keychain/keystore).
- **Projects**: model status at the top (tap it for Settings), add a project (name + folder path on the PC),
  long-press or ✕ to remove (only from Relay; the folder is never touched).
- **Sessions**: title, status badge and last activity; **New session** at the bottom; long-press to rename/delete.
- **Chat**: your prompts, the agent's answers in Markdown with code blocks, collapsible tool rows such as
  "Ran: cargo build — exit 0" (tap for arguments and output), and the model's thought process (collapsed).
  The input stays above the keyboard; **Send** becomes **Stop** while the agent runs. Updates arrive live over
  WebSocket, and the app reconnects and catches up when it returns to the foreground or the network comes back.
- **Settings**: server address and token, push toggle, restart model, server info, disconnect.

Checks: `npm run typecheck` (TypeScript) and `npx expo-doctor`.

## 9. Installable apps with EAS Build

Expo Go is for development. To install Relay as a normal app:

```powershell
cd Relay\mobile
npx eas-cli@latest login
npx eas-cli@latest init            # creates the EAS project and writes extra.eas.projectId to app.json
```

Change `ios.bundleIdentifier` and `android.package` in `app.json` (`com.example.relay`) to something of your own.

- **Android (APK you can sideload)**:

  ```powershell
  npx eas-cli@latest build --platform android --profile preview
  ```

  Open the link EAS prints on your phone, download the `.apk`, and install it (allow "install unknown apps").
- **iOS**: a permanent install needs a paid **Apple Developer account** (USD 99/year).

  ```powershell
  npx eas-cli@latest device:create                       # register your iPhone (ad hoc builds)
  npx eas-cli@latest build --platform ios --profile preview
  ```

  Without a paid account you can only run the app through Expo Go or a 7-day free provisioning build from Xcode
  on a Mac.

## 10. Push notifications

When a task finishes, errors or is stopped, Relay sends a push through Expo's push service containing only the
**project name, session title and status** (and ids so tapping it opens the session). Never code, file contents
or prompt text. Session titles default to the creation time for that reason; rename them freely.

To enable:

1. Run `npx eas-cli@latest init` in `mobile/` (push tokens need the EAS project id) and install an EAS build
   (section 9). Expo Go on Android does not support push notifications.
2. For Android builds, add Firebase Cloud Messaging credentials once:
   `npx eas-cli@latest credentials` → Android → Google Service Account / FCM V1 (follow the prompts).
   iOS push credentials are set up automatically by EAS.
3. In the app: **Settings → Notify when a task ends**. The app asks for permission and registers its token with Relay.

Turn it off server-wide with `[push] enabled = false` in config.toml.

---

## Configuration reference

`server/config.toml` (see `server/config.example.toml`):

```toml
token = "…"                 # API token; delete the line and restart to generate a new one
bind = "0.0.0.0:8765"       # reach only through Tailscale
system_prompt_path = "system_prompt.md"
skills_paths = []           # extra skill folders (added by `relay migrate`)
max_steps = 50              # model round-trips per prompt
shell = "powershell.exe"    # or "pwsh" for PowerShell 7

[llama]
llama_server_path = 'C:\llama.cpp'   # the folder or llama-server.exe itself
model_path = 'C:\Users\me\.lmstudio\models\…\model-Q6_K.gguf'
port = 8080
args = ["--jinja", "-ngl", "99", "-c", "65536", "--flash-attn", "on", "--cache-type-k", "q8_0",
        "--cache-type-v", "q8_0", "--temp", "0.6", "--top-k", "20", "--top-p", "0.95", "--host", "127.0.0.1"]
health_timeout_seconds = 600
model_name = "local"

[push]
enabled = true
```

Edit it while Relay is stopped. Set `RUST_LOG=relay=debug` for more logging.

## Instructions, AGENTS.md and skills

The system prompt for each task is built from:

1. `system_prompt.md` (global; edit freely),
2. an environment section (project name and folder, OS, date),
3. the project's `AGENTS.md`, if the project folder has one,
4. a list of available skills: each skill's `name` and `description` from its SKILL.md frontmatter and the path
   to its SKILL.md. The model reads the full file with `read_file` when a skill is relevant. Skill folders are
   readable but never writable by the agent.

Skills are discovered in `server\skills\` and every entry of `skills_paths` (a skill folder, or a folder of skill
folders). Changes are picked up on the next prompt.

## Testing without a GPU

`server/examples/mock_llama.rs` is a scripted stand-in for llama-server (it streams text and tool-call deltas
like the real thing). The test suite uses it; you can also run it to try the whole stack:

```powershell
cd Relay\server
cargo run --example mock_llama -- --port 8080      # terminal 1
cargo run -- serve                                 # terminal 2: finds the "model" already on 8080
```

Prompts mentioning `hello.txt` make it call `write_file`, `run: <command>` calls `run_command`, and anything
else is echoed.

## Security notes

- Every request and WebSocket needs the bearer token; tokens are compared in constant time.
- llama-server only listens on `127.0.0.1`. Relay listens on all interfaces so Tailscale can reach it: keep it off
  the public internet and restrict the firewall rule to `100.64.0.0/10`.
- File tools canonicalize every path (resolving `..`, symlinks and junctions) and refuse anything outside the
  project folder (skill folders are read-only).
- `run_command` refuses obviously destructive commands (formatting, `shutdown`, `rm -rf /`, deleting outside the
  project, …). This blocklist catches mistakes; it is **not** a sandbox. A command can still do anything your
  Windows user can, so only point Relay at projects you are happy for the agent to run code in.
- Push notifications contain only project name, session title and status.

## Known limitations

- Long sessions can exceed the model's context window (65,536 tokens by default); llama-server then returns an
  error, which Relay shows. Start a new session, or raise `-c` if VRAM allows.
- If Relay is killed abruptly (Task Manager, power loss), llama-server may keep running; the next start detects it
  on the port and reuses it. Tasks that were running are marked `stopped`.
- The command blocklist is string-based and best effort (see Security notes).
- The app is built for phones in portrait; tablets work but are not optimized.
- This repository was built and tested on Linux with a mock llama-server (the server was also type-checked for
  the Windows MSVC target); the first run on your PC with the real model is the end-to-end check.
