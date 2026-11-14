/** `idle`: unloaded to free the GPU; it loads when a chat is opened. */
export type ModelState = 'idle' | 'starting' | 'ready' | 'error';

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
  /** Always "brain": the computer that runs the model, agent and history. */
  role?: 'brain';
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
  /** A paired device this project's folder is on; null = the brain. */
  device_id?: string | null;
  /** Hidden from the lists until restored. */
  archived?: boolean;
  /** Which computer the folder is on (filled in by the brain). */
  location?: ProjectLocation;
}

/** Where a project's files live: the brain itself, or a paired computer. */
export type ProjectLocation =
  | { kind: 'none' }
  | { kind: 'brain'; name: string; online: boolean }
  | { kind: 'device'; device_id: string; name: string; online: boolean };

/** A paired device (e.g. a phone with Proot Linux) the agent can work in. */
export interface Device {
  id: string;
  name: string;
  /** Its own computer name. */
  hostname?: string;
  /** Only for "direct" devices (the brain calls them). */
  url?: string | null;
  /** "link": it connects to the brain (Relay app / `relay link`); "direct": `relay host`. */
  connection?: 'link' | 'direct';
  /** The address it is connected from (its Tailscale IP), while connected. */
  address?: string | null;
  os?: string;
  arch?: string;
  /** The folder it shares with the agent. */
  root?: string;
  shell?: string;
  last_seen?: string;
  /** The brain can reach it right now. */
  connected?: boolean;
}

/** A pairing code shown on the brain, typed on the new computer. */
export interface PairCode {
  code: string;
  expires_at: string;
  brain_url: string;
  brain_name: string;
  tailscale: boolean;
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
  role: 'user' | 'assistant' | 'tool' | 'summary' | 'continue';
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
  /** On an assistant message: which model wrote it, and how fast. */
  stats?: ReplyStats;
}

export interface ReplyStats {
  /** The model's file name. */
  model: string;
  /** Set when model routing picked the model. */
  route?: 'quick' | 'heavy';
  tokens?: number;
  tokens_per_sec?: number;
  prompt_per_sec?: number;
}

/** Sends quick questions to a small model and everything else to a big one. */
export interface RoutingSettings {
  enabled: boolean;
  quick_model: string;
  /** "" = the default model. */
  heavy_model: string;
  quick_max_words: number;
}

export interface MemoryUse {
  total_mb: number;
  used_mb: number;
  /** Used by llama-server, when it runs on that computer. */
  model_mb?: number;
}

export interface ModelSpeed {
  model: string;
  replies: number;
  tokens: number;
  avg_tokens_per_sec: number;
  last_tokens_per_sec: number;
  last_prompt_per_sec?: number | null;
  last_at: string;
}

export interface StatsResponse {
  brain: {
    name: string;
    os: string;
    model: { state: ModelStatus['state']; file: string | null };
    memory: MemoryUse | null;
    gpus: { name: string; total_mb: number; used_mb: number }[];
  };
  speeds: ModelSpeed[];
  devices: { id: string; name: string; os: string; connected: boolean; memory: MemoryUse | null }[];
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
  | { type: 'compacting'; active: boolean }
  | { type: 'loading_model'; active: boolean };

export interface PendingApproval {
  tool_call_id: string;
  name: string;
  arguments: string;
  requested_at: string;
  /** What "Always allow" saves for this call, when it can be covered by a rule. */
  suggested_rule?: RuleDraft | null;
}

export type ApprovalDecision = 'approve' | 'approve_all' | 'always_allow' | 'deny';

// ----- always-allow rules and activity -----

/** command: commands starting with these words; files: changes inside this folder; web: pages on this site. */
export type RuleKind = 'command' | 'files' | 'web';

export interface RuleDraft {
  kind: RuleKind;
  pattern: string;
  /** null = every project. */
  project_id?: string | null;
  /** e.g. "Commands starting with git status". */
  label?: string;
}

export interface ApprovalRule {
  id: number;
  kind: RuleKind;
  pattern: string;
  project_id: string | null;
  project_name?: string | null;
  created_at: string;
  uses: number;
  last_used_at: string | null;
}

/** How a tool call got to run, or didn't. */
export type AuditDecision = 'auto' | 'rule' | 'allowed' | 'allowed_all' | 'denied' | 'stopped' | 'off';

export interface AuditEntry {
  id: number;
  at: string;
  project_id: string;
  project_name: string;
  session_id: string;
  session_title: string;
  tool: string;
  /** The command, file, address or search. */
  detail: string;
  decision: AuditDecision;
  rule_id: number | null;
  is_error: boolean;
  /** False once the chat was deleted (the entry stays). */
  session_exists: boolean;
}

export type ActivityFilter = '' | 'commands' | 'files' | 'web' | 'asked' | 'denied';

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
  /** search_documents and read_document: the folders made searchable. */
  search_documents: boolean;
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
  /** Minutes unused before the model is unloaded; 0 keeps it loaded. */
  idle_unload_minutes: number;
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
  routing: RoutingSettings;
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

/** How a searchable folder's index stands (kept on the computer it's on). */
export interface FolderIndexStatus {
  path: string;
  exists: boolean;
  files: number;
  chunks: number;
  indexed_at: string | null;
  indexing: boolean;
  done: number;
  total: number;
  skipped: number;
  error: string | null;
}

/** A folder the agent can search, on the brain or a linked computer. */
export interface DocumentFolder {
  id: string;
  /** Empty for the brain. */
  device_id: string;
  path: string;
  created_at: string;
  computer: string;
  online: boolean;
  status: FolderIndexStatus;
}

/** One passage a document search found. */
export interface DocumentHit {
  computer: string;
  folder: string;
  path: string;
  rel: string;
  kind: 'text' | 'pdf' | 'docx';
  start: number;
  end: number;
  text: string;
  score: number;
}

export interface DocumentSearch {
  hits: DocumentHit[];
  notes: string[];
  indexing: boolean;
}

export interface FolderListing {
  path: string;
  root: string;
  home?: string;
  folders: string[];
}

/** When a scheduled task runs. Times are the brain's local time. */
export type Repeat =
  /** One time, at a local date and time ("2026-11-12T08:00"). */
  | { kind: 'once'; at: string }
  /** At a time of day ("08:00") on weekdays 1 (Monday) to 7 (Sunday); empty = every day. */
  | { kind: 'daily'; time: string; days: number[] }
  | { kind: 'every'; minutes: number };

export interface Schedule {
  id: string;
  project_id: string;
  project_name: string;
  name: string;
  prompt: string;
  repeat: Repeat;
  /** Every run continues one chat instead of starting a new one. */
  same_chat: boolean;
  /** Runs don't stop to ask before acting. */
  allow_all?: boolean;
  enabled: boolean;
  next_run: string | null;
  last_run: string | null;
  last_session_id: string | null;
  last_status: string | null;
  created_at: string;
}

export type QueueStatus = 'queued' | 'running' | 'done' | 'error' | 'stopped';

export interface QueueItem {
  id: string;
  project_id: string;
  project_name: string;
  session_id: string | null;
  session_title: string | null;
  schedule_id: string | null;
  /** The next message of a busy chat: sent when that chat is free, not in line. */
  follow_up?: boolean;
  /** Runs without stopping to ask before acting. */
  allow_all?: boolean;
  name: string;
  prompt: string;
  status: QueueStatus;
  error: string | null;
  created_at: string;
  started_at: string | null;
  finished_at: string | null;
  /** Running, and waiting for you to allow a tool call. */
  needs_approval?: boolean;
}

export interface QueueResponse {
  items: QueueItem[];
  queued: number;
  running: number;
}
