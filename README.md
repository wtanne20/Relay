# Relay

Relay runs coding agents on your own Windows or Linux PC, with your own GPU, and lets you use them from your
phone anywhere through Tailscale. Think of it as a Claude- or LM Studio Bionic-style agent chat, with the model
running at home and the chat in your pocket.

- **PC app (`server/`, `desktop/`)**: a Windows or Linux program (`relay.exe` / `relay`, inside the Relay desktop
  app) that sets up and supervises llama.cpp's `llama-server` with a local model, runs an agent loop with file and
  shell tools (PowerShell on Windows, `sh` on Linux) restricted to each project folder, keeps everything in SQLite,
  and serves a token-protected REST + WebSocket API.
- **Phone app (`mobile/`)**: an Expo (React Native) app for Android and iOS. Pair by scanning a QR code, manage
  projects and sessions, chat with the agent and watch it work live, approve its commands, stop it, tune every
  agent setting, and get a push notification when a task finishes or needs you.

Your phone reaches the PC **only over Tailscale**. llama-server listens on `127.0.0.1`, and on Windows the installer
opens Relay's port only to Tailscale addresses (on Linux, see [Linux](#linux) for the matching firewall rule).
Nothing is exposed to the internet.

```
 Phone app ──Tailscale──▶ Relay :8765 (REST + WebSocket, bearer token)
                            │
                            ├─ agent loop ─▶ llama-server 127.0.0.1:8080 (OpenAI-compatible, streaming)
                            ├─ tools: list_dir, read_file, write_file, edit_file, run_command (PowerShell or sh)
                            ├─ SQLite: projects, sessions, messages, push tokens
                            └─ Expo Push API (project name, session title, status only)
```

---

## Quick start

You need a PC running Windows 10/11 or a 64-bit Debian- or Ubuntu-based Linux (an NVIDIA GPU is best; on Windows
other GPUs work through Vulkan), a phone, and a free [Tailscale](https://tailscale.com) account.

1. **Tailscale.** Install it on the PC (<https://tailscale.com/download>) and on the phone (App Store or Play
   Store), and sign in to both with the same account.
2. **PC app.** Get it from the [latest release](../../releases/latest) (see [Downloads](#downloads)):
   - **Windows:** run `RelaySetup-<version>.exe`. Windows may say it protected your PC because the installer isn't
     code-signed: click **More info → Run anyway**. Keep the defaults and finish with **Open Relay now**.
   - **Linux:** install `Relay-<version>-amd64.deb` with `sudo apt install ./Relay-<version>-amd64.deb` and open
     **Relay** from your app menu (or run `relay-desktop`). On other distributions, make
     `Relay-<version>-x86_64.AppImage` executable (`chmod +x`) and run it. See [Linux](#linux) for what differs.
3. **First start.** Relay asks whether this computer is the brain; pick **This computer is the brain**. It finds
   your GPU, downloads llama.cpp, picks the largest model that fits in your VRAM and loads it. The model status
   shows at the bottom left of the Relay window.
4. **A model.** If none was found, open **Settings → Download models** and pick a coding model (for example a Qwen
   coder model, Q4_K_M or larger), or put a `.gguf` file you already have in Relay's `models` folder
   (`%LOCALAPPDATA%\Relay\models` on Windows, `~/.local/share/relay/models` on Linux). To use models another app
   already downloaded, such as LM Studio, add its folder under **Settings → Model libraries**. The home screen
   shows the pairing **QR code** (a larger one is under **Settings → Pair your phone**).
5. **Phone app.** Install Relay on the phone (see [Downloads](#downloads)), open it and tap **Scan QR code**. That's
   the whole connection setup. Pointing the phone's own camera app at the QR code works too.
6. **Use it.** Tap **New project** and enter a folder path on the PC (for example `C:\code\my-app` or
   `/home/me/code/my-app`), then pick it above the prompt on the home screen and ask for something. The phone
   remembers the project you picked last.

Relay keeps running in the tray when you close its window; use **Quit Relay** in the tray menu (or Settings) to stop
it and the model. To pair another phone or see the code again: **Relay → Settings → Pair your phone**.

### What the Windows installer does

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

Relay keeps its settings and data in `%LOCALAPPDATA%\Relay` on Windows and `~/.local/share/relay` on Linux
(`$XDG_DATA_HOME/relay` if that is set): `config.toml`, `relay.db`, `logs\`, `skills\`,
`models\`, `system_prompt.md`, `pairing.html` and the downloaded `llama.cpp\`.

| Step | How |
| --- | --- |
| GPU | `nvidia-smi` reports the GPU name, VRAM and the CUDA version the driver supports. |
| llama.cpp | Uses `llama_server_path` if set; else an existing `llama-server` (Relay's folder, `PATH`, `~/llama.cpp`, and on Windows also `C:\llama.cpp` and `Program Files\llama.cpp`); else downloads the latest official release from github.com/ggml-org/llama.cpp. On Windows that is the CUDA build matching your driver (plus its CUDA runtime) on NVIDIA, otherwise the Vulkan build. On Linux it is the x64 CPU build; see [Linux](#linux) to use your GPU. Progress shows in the app. |
| Model | Uses `model_path` if set; else scans Relay's `models` folder and any model libraries you added for `.gguf` files (skipping vision projectors and extra split parts) and picks the largest that fits in VRAM minus 2.5 GB. Change it any time in the app. |
| Skills | Reads skills from `%LOCALAPPDATA%\Relay\skills` and LM Studio's `%USERPROFILE%\.lmstudio\skills`. |
| Tailscale | `tailscale status` gives the PC's Tailscale IP, which goes into the QR code. |
| Token | A random 256-bit API token is generated and saved in `config.toml`. |

The choices are saved to `config.toml`, so later starts are quick. If something can't be set up automatically
(no model found, download blocked), the model bar in the app says what to do, and **Restart model** retries.
llama-server's output goes to `logs\llama-server.log`.

### Linux

The Linux app is the same Relay as on Windows (brain, desktop window, tray icon, pairing), with a few differences:

- **Downloads.** `Relay-<version>-amd64.deb` for Debian and Ubuntu, `Relay-<version>-x86_64.AppImage` for other
  x64 distributions, and `Relay-<version>-arm64.deb` for Debian or Ubuntu in Proot on an Android phone (there it
  runs without a tray, and closing the window quits).
- **GPU.** Relay only downloads llama.cpp's CPU build on Linux. To use your GPU, install or build a CUDA or Vulkan
  `llama-server` (see llama.cpp's build guide), then put it on your `PATH` or in `~/llama.cpp`, or set
  `llama_server_path` in `config.toml`, and click **Restart model**. On ARM64 there is no download at all, so
  install `llama-server` this way first.
- **Firewall.** There is no installer, so nothing opens or limits Relay's port for you. If you use a firewall,
  allow port 8765 from Tailscale only, for example with ufw:
  `sudo ufw allow in on tailscale0 to any port 8765 proto tcp`.
- **Starting.** Closing the window keeps Relay in the tray. To start it when you sign in, use your desktop's
  startup apps settings with `relay-desktop --minimized`.
- **Commands.** The agent's `run_command` tool uses `sh` instead of PowerShell, with the same project-folder rules
  and blocklist.

---

## Agent settings (from the phone)

**Settings → Agent settings** changes everything Relay and llama-server use, similar to LM Studio's model settings
and Bionic's agent settings. Edits are saved to `config.toml` on the PC when you tap **Save changes**.

| Section | Settings | Applies |
| --- | --- | --- |
| Model | Pick any GGUF found on the PC (with its size, and a warning if it may not fit in VRAM) | Reloads the model |
| Behaviour | **Ask before acting**: never, before commands, or before commands and file changes; **Thinking** on/off (reasoning models such as Qwen3); max steps per prompt; default command timeout | Next prompt |
| Tools | Turn each tool on or off: list folders, read files, write files, edit files, run commands | Next prompt |
| Sampling | Temperature, Top P, Top K, Min P, repeat penalty, presence penalty, max reply tokens, with **Reset** to defaults | Next prompt |
| Model loading | Context length, GPU layers, flash attention (on/auto/off), KV cache type for K and V, extra llama-server arguments, with **Reset** | Reloads the model |
| Prompt and skills | Edit the global **system prompt** (with reset to default); turn individual **skills** on or off | Next prompt |
| Notifications | Push notifications on or off for the PC | Immediately |

Each project also has **Instructions** (top right of a project's sessions list): notes added to the system prompt
for that project only, on top of the project's own `AGENTS.md`.

### Background tasks

**Tasks** (the calendar icon on the phone's home screen, or the sidebar on the desktop) lists the brain's queue and
scheduled tasks. Everything runs on the brain, with or without the app open:

- **Queue.** Add prompts with **Add to queue** (or the tray icon next to Send on the home screen). They run one
  after another, each in its own chat, so the GPU isn't shared between them. Move one to the front, cancel it, or
  stop it while it runs.
- **Scheduled tasks.** "Every weekday at 08:00, summarize what changed in this project." Daily (pick the days),
  every N minutes or hours, or once. Each run joins the queue, in a new chat or continuing one chat. Times are the
  brain's time zone. A run missed while the PC was off happens once when Relay starts again.
- **Messages while the agent works.** Type in a chat that's still working and tap the clock button: the message
  waits and is sent as soon as the current run ends.
- **Allow everything.** A queued or scheduled task follows **Ask before acting** like any chat, so one that needs
  your OK waits, and the queue waits with it. Turn on **Allow everything** for a task you trust and its commands
  and file changes run without asking.
- **Notifications.** You get the usual "finished", "failed" and "needs your approval" notifications. On the
  GitHub-built Android app (no push service) the phone also checks in with the PC every 15 minutes or so while the
  app is closed, so tasks that started while you were away still notify.

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

- **Windows installer**: *Actions → Windows installer* → the latest run → artifact **RelaySetup-&lt;version&gt;**.
  The workflow runs the test suite on Windows, builds `relay.exe` (statically linked, no Visual C++ runtime needed),
  builds the installer with Inno Setup, and installs and uninstalls it once as a check.
- **Linux app**: *Actions → Linux desktop* → artifacts **RelayLinux-amd64-&lt;version&gt;** (`.deb` and `.AppImage`,
  plus `relay-host-x86_64-linux.zip`) and **RelayLinux-arm64-&lt;version&gt;** (`.deb` for Proot). The workflow runs
  the test suite on Linux and builds the desktop app with Tauri.
- **Android app**: *Actions → Android app* → artifact **Relay-android** (`Relay.apk`). Open it on the phone and
  allow installing from that source when asked. It is signed with a development key, which is fine for your own
  phone.
- **Releases**: pushing a tag like `v0.1.0` attaches all of these to a GitHub Release.
- **iPhone**: Apple doesn't allow installing apps from a file. Build it with EAS (below) using a paid Apple
  Developer account, or run it in Expo Go.

---

## Building from source

### PC app

Install Rust (<https://rustup.rs>; on Windows accept the Visual Studio C++ Build Tools when rustup asks), then:

```powershell
cd Relay\server
cargo build --release
.\target\release\relay.exe          # same as: relay.exe serve
```

On Linux the same steps build `./target/release/relay`. The desktop window is built with Tauri; see
[DEVELOPING.md](DEVELOPING.md) and `.github/workflows/linux.yml` for the packages it needs.

`relay.exe` uses `%LOCALAPPDATA%\Relay\config.toml` (`~/.local/share/relay/config.toml` on Linux), unless there is a `config.toml` in the current folder or you
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

All settings live in `%LOCALAPPDATA%\Relay\config.toml` (`~/.local/share/relay/config.toml` on Linux). [`server/config.example.toml`](server/config.example.toml)
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
  Windows installer's firewall rule only admits Tailscale addresses (on Linux, add the rule in [Linux](#linux)).
  Never use Tailscale Funnel or port forwarding for it.
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
- Relay was developed against a mock llama-server on Linux and is built and tested on Windows and Linux by the CI
  workflows; the first run on your PC with a real model is the end-to-end check.
