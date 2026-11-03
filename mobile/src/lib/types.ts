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
}

export interface Project {
  id: string;
  name: string;
  path: string;
  created_at: string;
}

export type SessionStatus = 'idle' | 'running' | 'done' | 'error' | 'stopped';

export interface Session {
  id: string;
  project_id: string;
  title: string;
  status: SessionStatus;
  error: string | null;
  created_at: string;
  updated_at: string;
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
  role: 'user' | 'assistant' | 'tool';
  content: string;
  reasoning?: string;
  tool_calls?: ToolCall[];
  tool_call_id?: string;
  tool_name?: string;
  is_error?: boolean;
  created_at: string;
}

export interface Draft {
  content: string;
  reasoning: string;
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
    }
  | { type: 'status'; status: SessionStatus; error: string | null }
  | { type: 'text_delta'; text: string }
  | { type: 'reasoning_delta'; text: string }
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
  | { type: 'resync'; dropped: number };
