export type ModelState = 'starting' | 'ready' | 'error';

export interface ModelStatus {
  state: ModelState;
  message: string | null;
  external: boolean;
  pid: number | null;
  port: number;
  model_path: string;
  recent_restarts: number;
  since: string;
  /** 0..1 while llama.cpp is being downloaded on first run. */
  progress?: number | null;
  /** The loaded model has a vision file and can read pictures. */
  vision?: boolean;
  /** The model chats use when they don't pick one (may differ from the loaded one). */
  default_model?: string;
}

export interface GpuInfo {
  name: string;
  vram_mb: number;
}

export interface TailscaleInfo {
  dns_name: string | null;
  ip: string | null;
}

export interface ServerInfo {
  name: string;
  version: string;
  hostname: string;
  os: string;
  started_at: string;
  uptime_seconds: number;
  projects: number;
  push_enabled: boolean;
  max_steps: number;
  model: ModelStatus;
  gpus?: GpuInfo[];
  tailscale?: TailscaleInfo | null;
  /** When a phone last talked to this PC (null if never). */
  phone_last_seen?: string | null;
}

export interface Project {
  id: string;
  name: string;
  path: string;
  /** Extra instructions added to the system prompt for this project. */
  instructions?: string;
  created_at: string;
  /** Model file this project's chats use; null follows whatever is loaded. */
  model?: string | null;
  /** Hidden from the lists until restored. */
  archived?: boolean;
}

/** The built-in space for plain chats (no folder, no tools). */
export const CHATS_ID = 'chats';

export type SessionStatus = 'idle' | 'running' | 'done' | 'error' | 'stopped';

export interface Session {
  id: string;
  project_id: string;
  title: string;
  status: SessionStatus;
  error: string | null;
  created_at: string;
  updated_at: string;
  /** Model file for this chat; null follows the project. */
  model?: string | null;
  /** True while Relay may still name the chat. */
  auto_title?: boolean;
  /** Hidden from the lists until restored. */
  archived?: boolean;
}

export interface RecentSession extends Session {
  project_name: string;
}

export interface ToolCall {
  id: string;
  name: string;
  /** Raw JSON string produced by the model. */
  arguments: string;
}

export interface Message {
  id: number;
  session_id: string;
  role: 'user' | 'assistant' | 'tool' | 'summary';
  content: string;
  reasoning?: string;
  tool_calls?: ToolCall[];
  tool_call_id?: string;
  tool_name?: string;
  is_error?: boolean;
  created_at: string;
  /** On a summary: the newest message it replaces in the model's context. */
  covers_upto?: number;
  /** Pictures attached to a user message. */
  images?: string[];
  /** Other files attached to a user message. */
  files?: FileRef[];
}

export interface FileRef {
  name: string;
  stored: string;
  mime: string;
  size: number;
  /** "text", "pdf", "docx" (the model gets its text) or "other". */
  kind: string;
  chars: number;
}

export interface DraftCall {
  name: string;
  arguments: string;
}

export interface Draft {
  content: string;
  reasoning: string;
  /** Tool calls the model is still writing (for showing file edits live). */
  tool_calls?: DraftCall[];
}

export type ServerEvent =
  | {
      type: 'hello';
      session_id: string;
      status: SessionStatus;
      error: string | null;
      running: boolean;
      last_message_id: number | null;
      draft: Draft | null;
      pending_approval?: PendingApproval | null;
    }
  | { type: 'status'; status: SessionStatus; error: string | null }
  | { type: 'text_delta'; text: string }
  | { type: 'reasoning_delta'; text: string }
  | { type: 'tool_call_delta'; index: number; name: string; text: string }
  | { type: 'tool_call'; id: string; name: string; arguments: string }
  | {
      type: 'tool_result';
      tool_call_id: string;
      name: string;
      content: string;
      is_error: boolean;
      truncated: boolean;
    }
  | { type: 'message'; message: Message }
  | { type: 'final'; content: string }
  | { type: 'error'; message: string }
  | { type: 'resync'; dropped: number }
  | { type: 'approval_request'; approval: PendingApproval }
  | { type: 'approval_resolved'; tool_call_id: string; approved: boolean }
  | { type: 'title'; title: string }
  | { type: 'compacting'; active: boolean };

export interface PendingApproval {
  tool_call_id: string;
  name: string;
  arguments: string;
  requested_at: string;
}

export type ApprovalDecision = 'approve' | 'approve_all' | 'deny';

// ----- agent settings -----

export type ApprovalMode = 'never' | 'commands' | 'all_changes';

export interface ToolToggles {
  list_dir: boolean;
  read_file: boolean;
  write_file: boolean;
  edit_file: boolean;
  run_command: boolean;
  web_search: boolean;
  fetch_url: boolean;
}

export interface AgentSettings {
  max_steps: number;
  command_timeout_seconds: number;
  approval: ApprovalMode;
  thinking: boolean;
  /** Ask the model to keep its thinking short. */
  brief_thinking?: boolean;
  tools: ToolToggles;
  /** Brave Search API key; empty = DuckDuckGo. */
  search_api_key: string;
}

export interface SamplingSettings {
  temperature: number;
  top_p: number;
  top_k: number;
  min_p: number;
  repeat_penalty: number;
  presence_penalty: number;
  max_tokens: number;
}

export interface LlamaSettings {
  llama_server_path: string;
  model_path: string;
  models_dirs: string[];
  port: number;
  context_size: number;
  gpu_layers: number;
  flash_attn: 'on' | 'off' | 'auto';
  cache_type_k: string;
  cache_type_v: string;
  extra_args: string[];
  health_timeout_seconds: number;
  model_name: string;
  auto_setup: boolean;
}

export interface Settings {
  agent: AgentSettings;
  sampling: SamplingSettings;
  llama: LlamaSettings;
  push: { enabled: boolean };
  disabled_skills: string[];
  /** Folder new projects are created in ("" = not set). */
  projects_folder: string;
  /** Where Relay saves models; empty = its data folder. */
  models_folder?: string;
}

export interface SettingsResponse {
  settings: Settings;
  defaults: Settings;
  tools: { name: keyof ToolToggles; description: string }[];
  cache_types: string[];
  gpus: GpuInfo[];
}

export type DeepPartial<T> = {
  [K in keyof T]?: T[K] extends object ? (T[K] extends unknown[] ? T[K] : DeepPartial<T[K]>) : T[K];
};

export interface ModelFile {
  path: string;
  name: string;
  size_bytes: number;
  /** Has a vision file next to it, so it can read pictures. */
  vision?: boolean;
  /** What the model is good at, e.g. "code", "UI". */
  tags?: string[];
  /** False when the tags were guessed from the file name. */
  custom_tags?: boolean;
}

export interface SkillInfo {
  name: string;
  description: string;
  path: string;
  enabled: boolean;
}

export interface HubRepo {
  id: string;
  downloads: number;
  likes: number;
  updated?: string | null;
}

export interface HubFile {
  file: string;
  parts: string[];
  size_bytes: number;
  quant?: string | null;
}

export interface HubDownload {
  id: string;
  repo: string;
  file: string;
  dest: string;
  total_bytes: number;
  done_bytes: number;
  state: 'downloading' | 'done' | 'failed' | 'cancelled';
  error?: string | null;
}

export interface Disk {
  /** The drive, e.g. "W:\". */
  root: string;
  total_bytes: number;
  free_bytes: number;
  kind: 'local' | 'network' | 'removable' | 'unknown';
}

/** A folder Relay keeps models in, like a Steam library. */
export interface ModelLibrary {
  path: string;
  /** Relay's own folder in its data folder; always there. */
  builtin: boolean;
  /** New downloads go here. */
  default: boolean;
  exists: boolean;
  disk: Disk | null;
  models: { path: string; name: string; size_bytes: number }[];
  size_bytes: number;
}

export interface ReleaseAsset {
  name: string;
  url: string;
  size: number;
  sha256: string | null;
}

export interface Release {
  version: string;
  notes: string;
  page: string;
  published_at: string;
  windows: ReleaseAsset | null;
  android: ReleaseAsset | null;
  android_dev: ReleaseAsset | null;
}

export type InstallState =
  | { state: 'idle' }
  | { state: 'downloading'; done_bytes: number; total_bytes: number }
  | { state: 'installing' }
  | { state: 'failed'; error: string };

/** GET /updates */
export interface UpdateInfo {
  /** The Relay version running on the PC. */
  current: string;
  /** The PC's Relay is older than the latest release. */
  available: boolean;
  latest: Release | null;
  error: string | null;
  /** The PC can install updates itself (Windows). */
  can_install: boolean;
  install: InstallState;
}
