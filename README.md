# Relay

Relay runs coding agents on your own PC, with your own GPU, and lets you use them from your phone anywhere
through Tailscale. Think of it as a Claude- or LM Studio Bionic-style agent chat, with the model running at home
and the chat in your pocket.

- **PC app (`server/`)**: a Windows program (`relay.exe`) that sets up and supervises llama.cpp's
  `llama-server` with a local model, runs an agent loop with file and PowerShell tools restricted to each project
  folder, keeps everything in SQLite, and serves a token-protected REST + WebSocket API.
- **Phone app (`mobile/`)**: an Expo (React Native) app for Android and iOS. Pair by scanning a QR code, manage
  projects and sessions, chat with the agent and watch it work live, approve its commands, stop it, tune every
  agent setting, and get a push notification when a task finishes or needs you.

Your phone reaches the PC **only over Tailscale**. llama-server listens on `127.0.0.1`, and the installer opens
Relay's port only to Tailscale addresses. Nothing is exposed to the internet.

```
 Phone app ──Tailscale──▶ Relay :8765 (REST + WebSocket, bearer token)
                            │
                            ├─ agent loop ─▶ llama-server 127.0.0.1:8080 (OpenAI-compatible, streaming)
                            ├─ tools: list_dir, read_file, write_file, edit_file, run_command (PowerShell)
                            ├─ SQLite: projects, sessions, messages, push tokens
                            └─ Expo Push API (project name, session title, status only)
```

---

## Quick start

You need a Windows 10/11 PC (an NVIDIA GPU is best; other GPUs work through Vulkan), a phone, and a free
[Tailscale](https://tailscale.com) account.

1. **Tailscale.** Install it on the PC (<https://tailscale.com/download/windows>) and on the phone (App Store or
   Play Store), and sign in to both with the same account.
2. **A model.** If you use LM Studio, you're done: Relay uses the models LM Studio downloaded. Otherwise download a
   coding model in GGUF format (for example a Qwen coder model, Q4_K_M or larger) and put the `.gguf` file in
   `%LOCALAPPDATA%\Relay\models` after step 3.
3. **PC app.** Download `RelaySetup-<version>.exe` (see [Downloads](#downloads)) and run it. Windows may say it
   protected your PC because the installer isn't code-signed: click **More info → Run anyway**. Keep the defaults
   and finish with **Open Relay now**.
4. **First start.** Relay finds your GPU, downloads the right llama.cpp build for it, picks the largest model that
   fits in your VRAM and loads it. The model status shows at the bottom left of the Relay window. Click
   **Pair your phone** to see the **QR code**.
5. **Phone app.** Install Relay on the phone (see [Downloads](#downloads)), open it and tap **Scan QR code**. That's
   the whole connection setup. Pointing the phone's own camera app at the QR code works too.
6. **Use it.** Tap **Add project** and enter a folder path on the PC (for example `C:\code\my-app`), start a
   session, and ask for something.

Relay keeps running in the tray when you close its window; use **Quit Relay** in the tray menu (or Settings) to stop
it and the model. To pair another phone or see the code again: **Relay → Settings → Pair your phone**.

### What the installer does

- Installs the Relay Windows app (`relay-desktop.exe`, which runs the server and sits in the tray) and the
  `relay.exe` command-line tool into `Program Files\Relay`, with Start menu shortcuts: **Relay**,
  **Relay data folder** and **Uninstall Relay**.
- Optionally starts Relay in the tray when you sign in, and adds a desktop shortcut.
- Adds a Windows Firewall rule, *Relay (Tailscale only)*, allowing port 8765 **only from `100.64.0.0/10`**
  (Tailscale's address range). Uninstalling removes it.
- Tells you if Tailscale is missing and offers the download page.
- Uninstalling keeps your data (`%LOCALAPPDATA%\Relay`) unless you choose to delete it. Your project folders are
  never touched.

### What happens on first start

Relay keeps its settings and data in `%LOCALAPPDATA%\Relay`: `config.toml`, `relay.db`, `logs\`, `skills\`,
`models\`, `system_prompt.md`, `pairing.html` and the downloaded `llama.cpp\`.

| Step | How |
| --- | --- |
| GPU | `nvidia-smi` reports the GPU name, VRAM and the CUDA version the driver supports. |
| llama.cpp | Uses `llama_server_path` if set; else an existing `llama-server.exe` (Relay's folder, `PATH`, `%USERPROFILE%\llama.cpp`, `C:\llama.cpp`, `Program Files\llama.cpp`); else downloads the latest official release from github.com/ggml-org/llama.cpp: the CUDA build matching your driver (plus its CUDA runtime) on NVIDIA, otherwise the Vulkan build. Progress shows in the app. |
| Model | Uses `model_path` if set; else scans `%USERPROFILE%\.lmstudio\models`, LM Studio's older cache folder and `%LOCALAPPDATA%\Relay\models` for `.gguf` files (skipping vision projectors and extra split parts) and picks the largest that fits in VRAM minus 2.5 GB. Change it any time in the app. |
| Skills | Reads skills from `%LOCALAPPDATA%\Relay\skills` and LM Studio's `%USERPROFILE%\.lmstudio\skills`. |
| Tailscale | `tailscale status` gives the PC's Tailscale IP, which goes into the QR code. |
| Token | A random 256-bit API token is generated and saved in `config.toml`. |

The choices are saved to `config.toml`, so later starts are quick. If something can't be set up automatically
(no model found, download blocked), the model bar in the app says what to do, and **Restart model** retries.
llama-server's output goes to `logs\llama-server.log`.

---

## Agent settings (from the phone)

**Settings → Agent settings** changes everything Relay and llama-server use, similar to LM Studio's model settings
and Bionic's agent settings. Edits are saved to `config.toml` on the PC when you tap **Save changes**.

| Section | Settings | Applies |
| --- | --- | --- |
| Model | Pick any GGUF found on the PC (with its size, and a warning if it may not fit in VRAM) | Reloads the model |
| Behaviour | **Ask before acting**: never, before commands, or before commands and file changes; **Thinking** on/off (reasoning models such as Qwen3); max steps per prompt; default command timeout | Next prompt |
| Tools | Turn each tool on or off: list folders, read files, write files, edit files, run PowerShell | Next prompt |
| Sampling | Temperature, Top P, Top K, Min P, repeat penalty, presence penalty, max reply tokens, with **Reset** to defaults | Next prompt |
| Model loading | Context length, GPU layers, flash attention (on/auto/off), KV cache type for K and V, extra llama-server arguments, with **Reset** | Reloads the model |
| Prompt and skills | Edit the global **system prompt** (with reset to default); turn individual **skills** on or off | Next prompt |
| Notifications | Push notifications on or off for the PC | Immediately |

Each project also has **Instructions** (top right of a project's sessions list): notes added to the system prompt
for that project only, on top of the project's own `AGENTS.md`.

### Approvals

With **Ask before acting** set to *Commands* or *All changes*, the agent pauses before those tool calls. The chat
shows the exact command (or the file and the change) with **Allow**, **Deny** and **Allow everything for this
task**, and you get a push notification ("… needs your approval"). Denied calls are reported to the model, which
then tries another approach or asks you. Stopping the task while it waits cancels it.

### Share to Relay (Android)

Relay shows up in Android's share sheet. Share a link, some text, screenshots or photos, a voice note or any
other file from another app and Relay opens **Send to Relay**: add a note or tap a quick ask (**Summarize**,
**Explain**, **Key points**), pick a new chat (plain or in a project) or one of your recent chats, and send.
Pictures go to the model's vision and documents as text, as with the composer's **+** button. The model can't
listen to audio or watch video yet, so for a voice note it gets only the file's name and size. Files over 25 MB
are left out. iPhone sharing needs a separate share extension and isn't built yet.

### Voice mode (phone)

Tap the **voice** button in the composer (it shows while the message box is empty) to talk to the agent hands-free,
in a chat or from the home screen. Relay listens until you pause, sends what you said, waits for the agent, reads
the reply aloud and listens again. Approvals are read out too: say "allow", "deny" or "allow everything", or tap the
buttons. Say "goodbye" (or tap ✕) to leave; the chat keeps everything that was said.

Speech is turned into text and read aloud **on the phone** (Android's and iOS's own offline speech recognition and
text-to-speech), and voice prompts ask the model for short spoken answers instead of formatted text. If the phone
has no offline recognizer, or its language pack isn't downloaded, voice mode offers to download it (Android) or to
use the phone's online speech service instead, which you have to choose. Voice mode needs a phone app built from this version or later (it adds native speech
modules); it isn't offered in the desktop app.

---

## Downloads

Both apps are built by GitHub Actions in this repository:

- **PC installer**: *Actions → Windows installer* → the latest run → artifact **RelaySetup-&lt;version&gt;**. The
  workflow runs the test suite on Windows, builds `relay.exe` (statically linked, no Visual C++ runtime needed),
  builds the installer with Inno Setup, and installs and uninstalls it once as a check.
- **Android app**: *Actions → Android app* → artifact **Relay-android** (`Relay.apk`). Open it on the phone and
  allow installing from that source when asked. It is signed with a development key, which is fine for your own
  phone.
- **Releases**: pushing a tag like `v0.1.0` attaches both files to a GitHub Release.
- **iPhone**: Apple doesn't allow installing apps from a file. Build it with EAS (below) using a paid Apple
  Developer account, or run it in Expo Go.

---

## Building from source

### PC app

Install Rust (<https://rustup.rs>; accept the Visual Studio C++ Build Tools when rustup asks), then:

```powershell
cd Relay\server
cargo build --release
.\target\release\relay.exe          # same as: relay.exe serve
```

`relay.exe` uses `%LOCALAPPDATA%\Relay\config.toml`, unless there is a `config.toml` in the current folder or you
pass `--config <path>`. Commands:

- `relay serve`: run the server (default).
- `relay pair`: show the pairing QR code and open the pairing page.
- `relay migrate`: import from LM Studio Bionic (below).

To build the installer yourself, install [Inno Setup 6](https://jrsoftware.org/isinfo.php) and run
`iscc installer\relay.iss` after `cargo build --release`.

### Phone app

For development with Expo Go (Node.js LTS on the PC, Expo Go on the phone):

```powershell
cd Relay\mobile
npm install
npx expo start
```

For installable builds use EAS. Change `ios.bundleIdentifier` and `android.package` in `app.json`
(`com.example.relay`) to your own first.

```powershell
npx eas-cli@latest login
npx eas-cli@latest init                                        # writes extra.eas.projectId to app.json
npx eas-cli@latest build --platform android --profile preview  # APK
npx eas-cli@latest device:create                               # iPhone: register the device (ad hoc)
npx eas-cli@latest build --platform ios --profile preview      # needs a paid Apple Developer account
```

Checks: `npm run typecheck`.

---

## Migrate from LM Studio Bionic (`relay migrate`)

Relay already reads LM Studio's models and skills automatically. `relay migrate` also imports project folders and
your Bionic system prompt, without deleting, moving or modifying any original files. Bionic chat history is not
migrated. Run it while Relay is stopped.

```powershell
& "$env:ProgramFiles\Relay\relay.exe" migrate C:\code\app1 C:\code\app2 `
    --system-prompt C:\Users\me\Documents\bionic-prompt.txt

# Or scan a parent folder and confirm each subfolder (y/N):
& "$env:ProgramFiles\Relay\relay.exe" migrate --scan C:\code
```

1. **Projects**: each folder becomes a project named after the folder (already-imported folders are skipped).
2. **Skills**: skill folders under LM Studio's skills folder and any `--skills <path>` are referenced, never copied.
3. **System prompt**: with `--system-prompt <file>`, the text becomes `system_prompt.md` (an existing one is backed
   up to `system_prompt.md.bak`).
4. **Model**: lists the GGUF files LM Studio has and asks which one to use (Enter keeps the current one).

Other options: `--lmstudio <dir>`, `--llama-server <path>`, `--yes`, `--skip-model`. See `relay migrate --help`.

---

## Push notifications

When a task finishes, fails, is stopped or needs approval, Relay sends a push through Expo's push service with
only the **project name, session title and status** (plus ids so tapping it opens the session). Never code, file
contents or prompt text. Session titles default to the creation time for that reason.

Push needs an app built with EAS, because it needs the EAS project id (the GitHub-built APK and Expo Go don't
receive pushes):

1. `npx eas-cli@latest init` in `mobile/`, then build with EAS (above).
2. Android: add Firebase Cloud Messaging credentials once with `npx eas-cli@latest credentials`. iOS credentials
   are handled by EAS.
3. In the app: **Settings → Notifications on this phone**.

---

## Configuration reference

All settings live in `%LOCALAPPDATA%\Relay\config.toml`. [`server/config.example.toml`](server/config.example.toml)
lists every key with comments. Everything except `token`, `bind`, `shell` and `skills_paths` can be changed from
the app; edit the file only while Relay is stopped. `RUST_LOG=relay=debug` gives more logging. The HTTP API is
documented in [API.md](API.md).

## How the agent's instructions are built

For each task the system prompt is:

1. `system_prompt.md` (editable in the app),
2. an environment section (project name and folder, OS, date),
3. the project's `AGENTS.md`, if the folder has one,
4. the project's **Instructions** from the app,
5. the enabled skills: each skill's `name` and `description` from its `SKILL.md` frontmatter and the path to it.
   The model reads the full `SKILL.md` with `read_file` when relevant. Skill folders are readable but never
   writable by the agent.

## Testing without a GPU

`server/examples/mock_llama.rs` is a scripted stand-in for llama-server that streams text and tool-call deltas
like the real thing. The test suite uses it, and you can run it to try the whole stack:

```powershell
cd Relay\server
cargo run --example mock_llama -- --port 8080      # terminal 1
cargo run -- serve                                 # terminal 2: finds the "model" already on 8080
```

Prompts mentioning `hello.txt` make it call `write_file`, `run: <command>` calls `run_command`, and anything else
is echoed. `cargo run --example relay_cli -- --project <folder> "<prompt>"` is a small command-line client.

## Security notes

- Every request and WebSocket needs the bearer token; tokens are compared in constant time. The pairing QR code
  and `pairing.html` contain the token, so don't share them. To revoke it, delete the `token` line in
  `config.toml`, restart Relay and pair again.
- llama-server only listens on `127.0.0.1`. Relay listens on all interfaces so Tailscale can reach it, and the
  installer's firewall rule only admits Tailscale addresses. Never use Tailscale Funnel or port forwarding for it.
- Traffic is plain HTTP inside Tailscale's encrypted WireGuard tunnel, so the app allows cleartext HTTP.
- File tools canonicalize every path (resolving `..`, symlinks and junctions) and refuse anything outside the
  project folder; skill folders are read-only.
- `run_command` refuses obviously destructive commands (formatting, `shutdown`, `rm -rf /`, deleting outside the
  project, …). This blocklist catches mistakes; it is **not** a sandbox. Use **Ask before acting** if you want to
  review commands, and only point Relay at projects you're happy for the agent to run code in.

## Known limitations

- Long sessions can exceed the context window; llama-server then returns an error, which Relay shows. Start a new
  session or raise **Context length** if VRAM allows.
- If Relay is killed abruptly, llama-server may keep running; the next start reuses it. Interrupted tasks are
  marked `stopped`.
- The PC must stay awake while you use Relay from your phone.
- Relay was developed against a mock llama-server on Linux and is built and tested on Windows by the CI workflow;
  the first run on your PC with a real model is the end-to-end check.
