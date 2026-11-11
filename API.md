# Relay API

Relay listens on `0.0.0.0:8765` by default. Reach it **only over Tailscale**; never port-forward it.

- All bodies are JSON (`Content-Type: application/json`).
- **Every** request, including the WebSocket upgrade, needs `Authorization: Bearer <token>`.
  The token is generated on first run, saved in `config.toml` and printed in the console.
  WebSocket clients that cannot set headers (browsers) may pass `?token=<token>` instead.
- Errors always look like this, with a matching HTTP status
  (`400` bad input, `401` bad/missing token, `404` not found, `409` conflict, `500` internal):

```json
{ "error": "folder does not exist on the PC: C:\\code\\nope" }
```

Timestamps are RFC 3339 UTC strings. Ids are UUIDs, except message ids, which are increasing integers.

## Contents

- [Server](#server): `GET /info`, `GET /model`, `POST /model/restart`
- [Agent settings](#agent-settings): `GET/PATCH /settings`, `GET /models`, `GET/PUT /system-prompt`, `GET /skills`
- [Projects](#projects): `GET/POST /projects`, `GET/PATCH/DELETE /projects/{id}`
- [Sessions](#sessions): `GET/POST /projects/{id}/sessions`, `GET/PATCH/DELETE /sessions/{id}`
- [Messages and tasks](#messages-and-tasks): `GET /sessions/{id}/messages`, `POST /sessions/{id}/prompt`, `POST /sessions/{id}/stop`, `POST /sessions/{id}/approval`
- [Always-allow rules and activity](#always-allow-rules-and-activity): `GET/POST /approval-rules`, `DELETE /approval-rules/{id}`, `GET/DELETE /activity`
- [Linked computers](#linked-computers): `GET /hello`, `POST /devices/pair-code`, `POST /devices/claim`, `GET /devices/link`, `GET/PATCH/DELETE /devices/{id}`, `GET /devices/{id}/browse`
- [Push tokens](#push-tokens): `POST/DELETE /push-tokens`
- [WebSocket](#websocket-get-sessionsidws): `GET /sessions/{id}/ws` and every event type
- [Objects](#objects): Project, Session, Message, ModelStatus

---

## Server

### `GET /info`

Server and model overview.

```json
{
  "name": "relay",
  "version": "0.1.0",
  "hostname": "DESKTOP-PC",
  "os": "windows",
  "started_at": "2026-11-03T09:12:44.120+00:00",
  "uptime_seconds": 3600,
  "projects": 3,
  "push_enabled": true,
  "max_steps": 50,
  "model": { "...": "see ModelStatus" },
  "gpus": [ { "name": "NVIDIA GeForce RTX 5090", "vram_mb": 32607 } ],
  "tailscale": { "dns_name": "my-pc.tail1234.ts.net", "ip": "100.101.102.103" }
}
```

`gpus` is empty without an NVIDIA GPU; `tailscale` is `null` when Tailscale isn't running on the PC.

### `GET /model`

Returns a [ModelStatus](#modelstatus).

```json
{
  "state": "ready",
  "message": null,
  "external": false,
  "pid": 12345,
  "port": 8080,
  "model_path": "C:\\Users\\me\\.lmstudio\\models\\Qwen\\model-Q6_K.gguf",
  "recent_restarts": 0,
  "since": "2026-11-03T09:13:30.002Z"
}
```

`state` is `starting`, `ready` or `error`.

### `POST /model/restart`

Stops llama-server (if Relay launched it) and starts it again; also clears the
"gave up after 3 restarts" error state. Returns `202` immediately; poll `GET /model`.

```json
{ "ok": true }
```

---

## Agent settings

Everything here is saved to `config.toml` on the PC.

### `GET /settings`

```json
{
  "settings": {
    "agent": {
      "max_steps": 50,
      "command_timeout_seconds": 120,
      "approval": "never",
      "thinking": true,
      "tools": { "list_dir": true, "read_file": true, "write_file": true, "edit_file": true, "run_command": true }
    },
    "sampling": {
      "temperature": 0.6, "top_p": 0.95, "top_k": 20, "min_p": 0.0,
      "repeat_penalty": 1.0, "presence_penalty": 0.0, "max_tokens": 0
    },
    "llama": {
      "llama_server_path": "C:\\Users\\me\\AppData\\Local\\Relay\\llama.cpp\\b7000\\llama-server.exe",
      "model_path": "C:\\Users\\me\\.lmstudio\\models\\…\\model-Q6_K.gguf",
      "models_dirs": [], "port": 8080, "context_size": 65536, "gpu_layers": 99,
      "flash_attn": "on", "cache_type_k": "q8_0", "cache_type_v": "q8_0", "extra_args": [],
      "health_timeout_seconds": 600, "model_name": "local", "auto_setup": true
    },
    "push": { "enabled": true },
    "disabled_skills": []
  },
  "defaults": { "...": "the same shape with default values" },
  "tools": [ { "name": "run_command", "description": "Run PowerShell commands" } ],
  "cache_types": ["f16", "bf16", "q8_0", "q5_1", "q5_0", "q4_1", "q4_0"],
  "gpus": [ { "name": "NVIDIA GeForce RTX 5090", "vram_mb": 32607 } ]
}
```

- `agent.approval`: `never`, `commands` (ask before `run_command`) or `all_changes` (also before `write_file` and
  `edit_file`).
- `agent.thinking` is sent as `chat_template_kwargs.enable_thinking`.
- `sampling.max_tokens`: `0` means no limit.
- `llama.*` become llama-server arguments: `-m <model_path> --port <port> --jinja -ngl <gpu_layers>
  -c <context_size> --flash-attn <flash_attn> --cache-type-k <cache_type_k> --cache-type-v <cache_type_v>
  <extra_args…> --host 127.0.0.1`.

The token, bind address, shell and extra skill folders are not exposed and can't be changed through the API.

### `PATCH /settings`

A partial update: send only what changes, nested by section.

```json
{ "sampling": { "temperature": 0.2 }, "agent": { "approval": "commands" } }
```

```json
{ "settings": { "...": "all settings after the change" }, "model_restarting": false }
```

Changes to the `llama` section restart llama-server (`model_restarting: true`); everything else applies from the
next prompt. Unknown keys, wrong types, `null`, out-of-range values (for example temperature outside 0–2),
`--host`/`--port`/`-m` in `extra_args`, a `model_path` that isn't an existing `.gguf` file, and a
`llama_server_path` that doesn't exist are rejected with `400` and nothing is saved.

### `GET /models`

GGUF files found on the PC (LM Studio's folders, Relay's `models` folder, `llama.models_dirs`), plus the configured
model if it lives elsewhere. Select one with `PATCH /settings` `{"llama": {"model_path": "…"}}`.

```json
{
  "models": [ { "path": "C:\\…\\model-Q6_K.gguf", "name": "model-Q6_K.gguf", "size_bytes": 22548578304 } ],
  "current": "C:\\…\\model-Q6_K.gguf",
  "dirs": ["C:\\Users\\me\\AppData\\Local\\Relay\\models", "C:\\Users\\me\\.lmstudio\\models"],
  "vram_mb": 32607
}
```

### `GET /system-prompt`

```json
{ "content": "You are Relay, …", "default": "You are Relay, …", "path": "C:\\…\\system_prompt.md" }
```

### `PUT /system-prompt`

`{"content": "…"}` replaces `system_prompt.md` (max 200,000 bytes). Empty or whitespace-only content restores the
default. Returns `{"content": "…"}`.

### `GET /skills`

```json
{
  "skills": [ { "name": "pdf", "description": "Read PDFs", "path": "C:\\…\\pdf\\SKILL.md", "enabled": true } ],
  "roots": ["C:\\…\\Relay\\skills", "C:\\Users\\me\\.lmstudio\\skills"]
}
```

Turn skills off with `PATCH /settings` `{"disabled_skills": ["pdf"]}`.

---

## Projects

A project is a name plus an absolute folder path. The folder is on the brain, or on a
[linked computer](#linked-computers) when `device_id` is set. The agent's tools are confined to that folder.

Every project in a response carries `location`, which computer its folder is on:

```json
{ "kind": "brain", "name": "DESKTOP-WILL", "online": true }
{ "kind": "device", "device_id": "c687…", "name": "laptop", "online": false }
{ "kind": "none" }
```

(`none` is the built-in plain-chats space, which has no folder.)

### `GET /projects`

```json
{
  "projects": [
    { "id": "8f1c…", "name": "my-app", "path": "C:\\code\\my-app", "created_at": "2026-11-03T09:20:00.000Z" }
  ]
}
```

### `POST /projects` → `201`

`name` is optional (defaults to the folder name). The path must be absolute and exist; it is stored canonicalized.

```json
{ "name": "My app", "path": "C:\\code\\my-app" }
```

```json
{ "project": { "id": "8f1c…", "name": "My app", "path": "C:\\code\\my-app", "created_at": "…" } }
```

`409` if the folder is already a project.

For a folder on a linked computer, add `device_id`; `path` is then an absolute path on that computer, inside the
folder it shares (empty: the shared folder itself). The computer must be connected; it checks the folder.

```json
{ "device_id": "c687…", "path": "/home/will/code/website" }
```

### `GET /projects/{id}`

```json
{ "project": { "id": "8f1c…", "name": "My app", "path": "C:\\code\\my-app", "instructions": "", "created_at": "…" } }
```

### `PATCH /projects/{id}`

All fields optional. `instructions` (max 50,000 bytes) is added to the system prompt for this project's sessions.

```json
{ "name": "Renamed", "path": "D:\\work\\my-app", "instructions": "Run npm test before finishing." }
```

```json
{ "project": { "id": "8f1c…", "name": "Renamed", "path": "D:\\work\\my-app", "created_at": "…" } }
```

### `DELETE /projects/{id}`

Removes the project, its sessions and messages **from Relay only**. The folder on disk is never touched.
`409` while a task is running in one of its sessions.

```json
{ "ok": true }
```

---

## Sessions

### `GET /projects/{id}/sessions`

Newest activity first.

```json
{
  "sessions": [
    {
      "id": "c2d7…",
      "project_id": "8f1c…",
      "title": "Fix the build",
      "status": "done",
      "error": null,
      "created_at": "2026-11-03T09:21:00.000Z",
      "updated_at": "2026-11-03T09:25:41.380Z"
    }
  ]
}
```

`status` is `idle`, `running`, `done`, `error` or `stopped`. `error` holds the message when status is `error`.

### `POST /projects/{id}/sessions` → `201`

The body is optional. Without a title the session is named after the current time
(titles are sent in push notifications, so they are never derived from prompt text).

```json
{ "title": "Fix the build" }
```

```json
{ "session": { "id": "c2d7…", "project_id": "8f1c…", "title": "Fix the build", "status": "idle", "error": null, "created_at": "…", "updated_at": "…" } }
```

### `GET /sessions/{id}`

```json
{ "session": { "id": "c2d7…", "title": "Fix the build", "status": "running", "...": "…" } }
```

### `PATCH /sessions/{id}` (rename)

```json
{ "title": "Fix the release build" }
```

```json
{ "session": { "id": "c2d7…", "title": "Fix the release build", "...": "…" } }
```

### `DELETE /sessions/{id}`

`409` while its task is running.

```json
{ "ok": true }
```

---

## Messages and tasks

### `GET /sessions/{id}/messages`

Paginated, always returned in ascending `id` order.

| Query | Meaning |
|---|---|
| *(none)* | the newest `limit` messages |
| `before=<id>` | the newest `limit` messages older than `id` (scroll back) |
| `after=<id>` | the oldest `limit` messages newer than `id` (catch up after reconnect) |
| `limit` | 1 to 500, default 50 |

`has_more` tells whether more messages exist in the paging direction.

```json
{
  "messages": [
    { "id": 41, "session_id": "c2d7…", "role": "user", "content": "create hello.txt containing hi", "created_at": "…" },
    {
      "id": 42, "session_id": "c2d7…", "role": "assistant",
      "content": "I'll create the file.",
      "reasoning": "The user wants a file…",
      "tool_calls": [ { "id": "call_1", "name": "write_file", "arguments": "{\"path\":\"hello.txt\",\"content\":\"hi\"}" } ],
      "created_at": "…"
    },
    {
      "id": 43, "session_id": "c2d7…", "role": "tool",
      "content": "wrote hello.txt (2 bytes)",
      "tool_call_id": "call_1", "tool_name": "write_file",
      "created_at": "…"
    },
    { "id": 44, "session_id": "c2d7…", "role": "assistant", "content": "Created hello.txt.", "created_at": "…" }
  ],
  "has_more": false
}
```

### `POST /sessions/{id}/prompt` → `202`

Stores the prompt and starts the agent as a background task (it keeps running if the phone disconnects).
`409` if a task is already running in this session; `400` for an empty prompt.

```json
{ "content": "Run the tests and fix any failures" }
```

```json
{
  "message": { "id": 45, "session_id": "c2d7…", "role": "user", "content": "Run the tests and fix any failures", "created_at": "…" },
  "status": "running"
}
```

The task loops (model → tools → model) until the model answers without tool calls or 50 steps pass.
Its end state is `done`, `error` or `stopped`.

### `POST /sessions/{id}/stop`

Cancels the running task, including killing a running command and its child processes.
`stopping` is `false` if nothing was running.

```json
{ "stopping": true }
```

---

### `POST /sessions/{id}/approval`

Answers the tool call the agent is waiting on (see `approval_request` below).

```json
{ "tool_call_id": "call_3", "decision": "approve", "note": null }
```

`decision` is `approve`, `approve_all` (this call and every later one in the current task), `always_allow` or
`deny`. With `deny`, an optional `note` is passed to the model as the reason. Returns `{"ok": true}`, or `409` if
nothing is waiting or the id doesn't match.

`always_allow` approves the call and saves an always-allow rule (see below) so calls like it aren't asked about
again. The rule is the request's `suggested_rule` unless the body carries its own `rule`. The saved rule comes
back as `"rule"`.

## Always-allow rules and activity

A rule skips the question for tool calls it covers. Rules never run a tool that is turned off, and only matter
when `agent.approval` asks about something.

| `kind` | Covers | `pattern` |
| --- | --- | --- |
| `command` | `run_command` | Words the command starts with: `git status` covers `git status -s`, not `git statusx` or `git push`. A command with `; & \| $ \` < > ( ) { }` or a line break is never covered. |
| `files` | `write_file`, `edit_file` | A folder in the project (`src`, or `.` for all of it). Always tied to one project. Paths with `..` are never covered. |
| `web` | `fetch_url` | A site: `docs.rs` covers `docs.rs` and its subdomains. |

`project_id` is the project the rule applies to, or `null` for every project.

### `GET /approval-rules`

`{"rules": [{"id": 3, "kind": "command", "pattern": "git status", "project_id": "…", "project_name": "app", "created_at": "…", "uses": 12, "last_used_at": "…"}]}`

### `POST /approval-rules`

Body `{"kind": "web", "pattern": "https://www.docs.rs/serde", "project_id": null}`. Returns the saved rule
(the pattern is tidied, here to `docs.rs`). Saving the same rule twice returns the existing one. `400` for a
pattern that can't be a rule.

### `DELETE /approval-rules/{id}`

`{"ok": true}`, or `404`.

### `GET /activity`

Every tool call the agent made, newest first, and how it got to run. Entries stay when a chat is deleted.
Query: `before` (an entry id, for the next page), `limit` (default 50, at most 200), `project_id`, `session_id`,
and `filter`: `commands`, `files` (writes and edits), `web`, `asked` (you answered) or `denied`.

```json
{
  "entries": [
    {
      "id": 812, "at": "…", "project_id": "…", "project_name": "app", "session_id": "…", "session_title": "Fix build",
      "tool": "run_command", "detail": "git status -s", "decision": "rule", "rule_id": 3, "is_error": false,
      "session_exists": true
    }
  ],
  "has_more": true
}
```

`detail` is the command, path, address or search, never file contents. `decision` is `auto` (nothing asked),
`rule`, `allowed`, `allowed_all`, `denied`, `stopped` or `off` (the tool is turned off). The newest 20,000
entries are kept.

### `DELETE /activity`

Clears the log: `{"removed": 812}`.

## Linked computers

The brain runs the model, the agent and the history. A linked computer shares one folder with it, and projects
can keep their files there. The linked computer dials out to the brain over Tailscale and keeps one WebSocket
open; the brain sends file and command requests down it. Nothing listens on the linked computer.

### `GET /hello` (no token)

Lets a computer on the tailnet find brains. `{"relay": "brain", "name": "DESKTOP-WILL", "version": "1.0.6", "os": "windows"}`

### `POST /devices/pair-code`

Starts pairing: `{"code": "532397", "expires_at": "…", "brain_url": "http://100.101.102.103:8765", "brain_name": "…", "tailscale": true}`.
The code works once, for ten minutes, and is dropped after five wrong tries.

### `POST /devices/claim` (no token; the code is the proof)

```json
{ "code": "532397", "name": "laptop", "hostname": "will-laptop", "os": "linux", "arch": "x86_64",
  "root": "/home/will", "previous_id": null }
```

`201` with `{"device_id", "device_key", "api_token", "brain_name"}`. `device_key` is this computer's own key for
the link; removing the device revokes it. `api_token` lets the computer's Relay window open the brain's chats.
`previous_id` (the id from an earlier pairing with the same brain) keeps the id, so its projects stay bound.

### `GET /devices/link` (WebSocket, `Authorization: Bearer <device_key>`)

The link. Frames are JSON text: the brain sends `{"t":"req","id":7,"m":"GET","p":"/list?path=.","f":"/home/will/app","b":"<base64>"}`
(`f` is the project folder, `b` the body) and the computer answers `{"t":"res","id":7,"s":200,"b":"<base64>"}`.
The paths are the `relay host` endpoints (`/info`, `/list`, `/read`, `/write`, `/edit`, `/run`, `/run/{id}`).
Both sides ping every 15 s and drop the link after 45 s of silence; the computer reconnects with backoff.

### `GET /devices`

```json
{ "devices": [ { "id": "c687…", "name": "laptop", "hostname": "will-laptop", "connection": "link",
  "connected": true, "address": "100.64.0.7", "os": "linux", "arch": "x86_64", "root": "/home/will",
  "shell": "sh", "last_seen": "…", "url": null } ] }
```

`connection` is `link` (dials in) or `direct` (an older `relay host` the brain calls at `url`, added with
`POST /devices` and `{name, url, token}`).

### `PATCH /devices/{id}`, `DELETE /devices/{id}`

Rename (`{"name": "…"}`), or forget the computer and revoke its key (its files are not touched).

### `GET /devices/{id}/browse?path=`

Sub-folders of a folder on the computer, for choosing a project folder: `{"path", "root", "folders": ["code", …]}`.

## Push tokens

Expo push tokens to notify when a task finishes, errors or is stopped. Notifications contain only the
project name, session title and status (plus ids for navigation), never code, file contents or prompts.

### `POST /push-tokens` → `201`

```json
{ "token": "ExponentPushToken[xxxxxxxxxxxxxxxxxxxxxx]" }
```

```json
{ "ok": true, "push_enabled": true }
```

### `DELETE /push-tokens`

```json
{ "token": "ExponentPushToken[xxxxxxxxxxxxxxxxxxxxxx]" }
```

```json
{ "ok": true, "removed": true }
```

What Relay sends to `https://exp.host/--/api/v2/push/send`:

```json
[{
  "to": "ExponentPushToken[…]",
  "title": "My app",
  "body": "Fix the build finished",
  "sound": "default",
  "priority": "high",
  "channelId": "tasks",
  "data": { "project_id": "8f1c…", "session_id": "c2d7…", "status": "done" }
}]
```

---

## WebSocket: `GET /sessions/{id}/ws`

```
ws://my-pc.tail1234.ts.net:8765/sessions/c2d7…/ws
Authorization: Bearer <token>
```

Each frame is one JSON event with a `type`. The server pings every 25 s; a client may also send the text
`ping` and receives `pong`. Events for a session are fanned out with a broadcast channel, so any number of
clients can watch the same session.

### Reconnecting

1. Open the WebSocket first (so nothing is missed), then
2. fetch history with `GET /sessions/{id}/messages?after=<newest id you have>` (or without `after` on first load),
3. upsert every `message` event by `id` (duplicates are expected and harmless).

The `hello` event carries the current status and, if a task is running, the partial answer streamed so far.

### Events

#### `hello` (first frame after connecting)

```json
{
  "type": "hello",
  "session_id": "c2d7…",
  "status": "running",
  "running": true,
  "error": null,
  "last_message_id": 44,
  "draft": { "content": "Looking at the failing", "reasoning": "The tests fail because…" },
  "pending_approval": null
}
```

`draft` is `null` when no task is running. `pending_approval` is the waiting tool call (same shape as in
`approval_request`) or `null`.

#### `status`

Sent when a task starts and when it ends.

```json
{ "type": "status", "status": "running", "error": null }
```

```json
{ "type": "status", "status": "error", "error": "model is not available: llama-server not found at C:\\llama\\llama-server.exe" }
```

#### `text_delta`

A piece of the assistant's answer as it streams.

```json
{ "type": "text_delta", "text": "I'll run the " }
```

#### `reasoning_delta`

A piece of the model's thinking (Qwen3 reasoning), streamed separately.

```json
{ "type": "reasoning_delta", "text": "The user wants me to" }
```

#### `tool_call` (tool call started)

`arguments` is the raw JSON string from the model.

```json
{ "type": "tool_call", "id": "call_7", "name": "run_command", "arguments": "{\"command\":\"cargo test\",\"timeout_seconds\":300}" }
```

#### `tool_result`

The result, cut to 2,000 characters (`truncated: true`); the full result is in the stored `tool` message.

```json
{
  "type": "tool_result",
  "tool_call_id": "call_7",
  "name": "run_command",
  "content": "exit_code: 0\n--- stdout ---\nrunning 12 tests\n…",
  "is_error": false,
  "truncated": true
}
```

#### `message`

Every stored message (the user prompt, each assistant turn, each tool result) as soon as it is saved.
Use it to replace the streaming draft with the final text.

```json
{ "type": "message", "message": { "id": 46, "session_id": "c2d7…", "role": "assistant", "content": "All 12 tests pass now.", "created_at": "…" } }
```

#### `final`

The final answer (the assistant turn with no tool calls). Followed by `status: done`.

```json
{ "type": "final", "content": "All 12 tests pass now." }
```

#### `error`

A task error, followed by `status: error`.

```json
{ "type": "error", "message": "llama-server returned 400 Bad Request: the request exceeds the available context size" }
```

#### `resync`

The client fell behind and `dropped` events were skipped; refetch history with `?after=`.

```json
{ "type": "resync", "dropped": 120 }
```

#### `approval_request`

The agent is waiting for permission to run a tool call. Answer with `POST /sessions/{id}/approval`.

```json
{
  "type": "approval_request",
  "approval": {
    "tool_call_id": "call_3", "name": "run_command", "arguments": "{\"command\":\"npm test\"}", "requested_at": "…",
    "suggested_rule": { "kind": "command", "pattern": "npm test", "project_id": "…", "label": "Commands starting with npm test" }
  }
}
```

`suggested_rule` is what `always_allow` saves, or `null` when the call can't be covered by a rule.

#### `approval_resolved`

```json
{ "type": "approval_resolved", "tool_call_id": "call_3", "approved": true }
```

### Typical sequence

```
hello → status(running) → message(user) → reasoning_delta… → text_delta… → message(assistant with tool_calls)
      → tool_call → tool_result → message(tool) → text_delta… → message(assistant) → final → status(done)
```

---

## Tools the agent can use

| Tool | Arguments | Notes |
|---|---|---|
| `list_dir` | `path` | Directories end with `/`; sizes in bytes; up to 1000 entries. |
| `read_file` | `path`, optional `start_line`, `end_line` | Up to 50,000 characters. Also reads SKILL.md files (read-only). |
| `write_file` | `path`, `content` | Creates parent folders. |
| `edit_file` | `path`, `old_text`, `new_text` | Exact match; error if not found or not unique. |
| `run_command` | `command`, `timeout_seconds` (default from settings, 120) | PowerShell in the project folder; killed on timeout or stop; exit code + stdout + stderr, truncated to 20,000 characters. |

Tools can be turned off in the settings; disabled tools aren't offered to the model. Paths are resolved against the project folder, canonicalized (following symlinks and junctions) and rejected
if they end up outside it. Obviously destructive commands (formatting disks, `shutdown`, `rm -rf /`,
`Remove-Item` outside the project, …) are refused. That blocklist is a safety net against model mistakes,
not a sandbox.

## Objects

### Project

| Field | Type |
|---|---|
| `id` | string (UUID) |
| `name` | string |
| `path` | string, absolute |
| `instructions` | string, extra system prompt text for this project |
| `created_at` | string |

### Session

| Field | Type |
|---|---|
| `id`, `project_id` | string |
| `title` | string |
| `status` | `idle` \| `running` \| `done` \| `error` \| `stopped` |
| `error` | string or null |
| `created_at`, `updated_at` | string |

### Message

| Field | Type | When |
|---|---|---|
| `id` | integer | always |
| `session_id` | string | always |
| `role` | `user` \| `assistant` \| `tool` | always |
| `content` | string | always (may be empty for tool-only assistant turns) |
| `reasoning` | string | assistant, if the model produced reasoning |
| `tool_calls` | `[{id, name, arguments}]` | assistant turns that call tools |
| `tool_call_id`, `tool_name` | string | tool results |
| `is_error` | boolean (omitted when false) | tool results |
| `created_at` | string | always |

### ModelStatus

| Field | Type |
|---|---|
| `state` | `starting` \| `ready` \| `error` |
| `message` | string or null: progress or error detail |
| `external` | true if Relay is using a llama-server it did not start |
| `pid` | number or null |
| `port` | number |
| `model_path` | string |
| `recent_restarts` | unexpected exits in the last 5 minutes (gives up at 3) |
| `since` | when the state last changed |
| `progress` | 0–1 while llama.cpp is downloading on first run, else null |
