import type { Credentials } from './storage';
import type {
  HubDownload,
  InstallState,
  UpdateInfo,
  ModelLibrary,
  HubFile,
  HubRepo,
  ApprovalDecision,
  DeepPartial,
  Device,
  PairCode,
  Message,
  ModelFile,
  ModelStatus,
  Project,
  ServerInfo,
  Session,
  Settings,
  SettingsResponse,
  SkillInfo,
  RecentSession,
} from './types';

export const DEFAULT_PORT = 8765;
const REQUEST_TIMEOUT_MS = 15000;

export type ErrorKind = 'network' | 'timeout' | 'auth' | 'http';

export class ApiError extends Error {
  constructor(
    message: string,
    public kind: ErrorKind,
    public status?: number,
  ) {
    super(message);
    this.name = 'ApiError';
  }
}

/**
 * Accepts "100.64.1.2", "my-pc", "my-pc.tail1234.ts.net:8765" or a full
 * http(s) URL and returns e.g. "http://my-pc:8765".
 */
export function normalizeServerUrl(input: string): string {
  let s = input.trim().replace(/\/+$/, '');
  if (!s) return '';
  if (!/^https?:\/\//i.test(s)) s = `http://${s}`;
  const m = s.match(/^(https?:\/\/)([^/]+)(.*)$/i);
  if (!m) return s;
  const [, scheme, hostPort, rest] = m;
  const hasPort = /:\d+$/.test(hostPort) || /^\[.*\]:\d+$/.test(hostPort);
  return `${scheme.toLowerCase()}${hasPort ? hostPort : `${hostPort}:${DEFAULT_PORT}`}${rest}`;
}

export function describeError(e: unknown): string {
  if (e instanceof ApiError) return e.message;
  if (e instanceof Error) return e.message;
  return String(e);
}

export class RelayClient {
  constructor(public creds: Credentials) {}

  get host(): string {
    return this.creds.url.replace(/^https?:\/\//, '');
  }

  wsUrl(sessionId: string): string {
    return `${this.creds.url.replace(/^http/i, 'ws')}/sessions/${encodeURIComponent(sessionId)}/ws`;
  }

  async request<T>(method: string, path: string, body?: unknown, timeoutMs = REQUEST_TIMEOUT_MS): Promise<T> {
    const controller = new AbortController();
    const timer = setTimeout(() => controller.abort(), timeoutMs);
    let res: Response;
    try {
      res = await fetch(`${this.creds.url}${path}`, {
        method,
        headers: {
          Authorization: `Bearer ${this.creds.token}`,
          Accept: 'application/json',
          ...(body !== undefined ? { 'Content-Type': 'application/json' } : {}),
        },
        body: body !== undefined ? JSON.stringify(body) : undefined,
        signal: controller.signal,
      });
    } catch (e) {
      const aborted = controller.signal.aborted;
      throw new ApiError(
        aborted
          ? `${this.host} did not respond in ${Math.round(timeoutMs / 1000)}s. Is Relay running and Tailscale connected?`
          : `Can't reach ${this.host}. Check that Tailscale is connected on both devices and Relay is running.`,
        aborted ? 'timeout' : 'network',
      );
    } finally {
      clearTimeout(timer);
    }
    let data: unknown = null;
    const text = await res.text();
    if (text) {
      try {
        data = JSON.parse(text);
      } catch {
        data = null;
      }
    }
    if (res.status === 401) {
      throw new ApiError(
        'The PC rejected the token. Scan the QR code again (Relay on your PC → Settings → Pair your phone).',
        'auth',
        401,
      );
    }
    if (!res.ok) {
      const msg =
        data && typeof data === 'object' && 'error' in data && typeof (data as { error: unknown }).error === 'string'
          ? (data as { error: string }).error
          : `Request failed (${res.status})`;
      throw new ApiError(msg, 'http', res.status);
    }
    return data as T;
  }

  // ----- server -----
  info() {
    return this.request<ServerInfo>('GET', '/info');
  }
  /** Pairing details for a phone: address, relay:// link and QR code (SVG). */
  pairing() {
    return this.request<{ url: string; link: string; svg: string; tailscale: boolean }>('GET', '/pairing');
  }
  model() {
    return this.request<ModelStatus>('GET', '/model');
  }
  restartModel() {
    return this.request<{ ok: boolean }>('POST', '/model/restart');
  }
  loadModel() {
    return this.request<{ ok: boolean }>('POST', '/model/load');
  }
  /** Frees the GPU; the model loads again when a chat is opened. */
  unloadModel() {
    return this.request<{ ok: boolean }>('POST', '/model/unload');
  }

  // ----- agent settings -----
  settings() {
    return this.request<SettingsResponse>('GET', '/settings');
  }
  patchSettings(patch: DeepPartial<Settings>) {
    return this.request<{ settings: Settings; model_restarting: boolean }>('PATCH', '/settings', patch);
  }
  hubSearch(q: string) {
    return this.request<{ results: HubRepo[] }>('GET', `/hub/search?q=${encodeURIComponent(q)}`, undefined, 30000);
  }
  hubFiles(repo: string) {
    return this.request<{ repo: string; files: HubFile[]; vram_mb: number }>(
      'GET',
      `/hub/files?repo=${encodeURIComponent(repo)}`,
      undefined,
      30000,
    );
  }
  hubDownloads() {
    return this.request<{ downloads: HubDownload[] }>('GET', '/hub/downloads');
  }
  hubDownload(repo: string, file: string, library?: string) {
    return this.request<{ download: HubDownload }>('POST', '/hub/downloads', { repo, file, library }, 30000);
  }
  libraries() {
    return this.request<{ libraries: ModelLibrary[]; current_model: string }>('GET', '/libraries');
  }
  /** Adds a folder (created if needed) as a model library. */
  addLibrary(path: string, makeDefault: boolean) {
    return this.request<{ libraries: ModelLibrary[] }>('POST', '/libraries', { path, make_default: makeDefault });
  }
  setDefaultLibrary(path: string) {
    return this.request<{ libraries: ModelLibrary[] }>('POST', '/libraries/default', { path });
  }
  /** Forgets a library; its files stay on the disk. */
  removeLibrary(path: string) {
    return this.request<{ libraries: ModelLibrary[] }>('POST', '/libraries/remove', { path });
  }
  /** Moves a model to another library; progress shows in hubDownloads (repo "move"). */
  moveModel(path: string, library: string) {
    return this.request<{ download: HubDownload }>('POST', '/models/move', { path, library }, 30000);
  }
  hubCancel(id: string) {
    return this.request<{ ok: boolean }>('DELETE', `/hub/downloads/${encodeURIComponent(id)}`);
  }
  /** Copies a .gguf file from elsewhere on the PC into Relay's own models folder. */
  importModel(path: string) {
    return this.request<{ download: HubDownload }>('POST', '/models/import', { path });
  }
  /** Sets a model's tags; null goes back to the tags guessed from its name. */
  setModelTags(name: string, tags: string[] | null) {
    return this.request<{ tags: string[] }>('PUT', '/models/tags', { name, tags });
  }
  models() {
    return this.request<{ models: ModelFile[]; current: string; dirs: string[]; vram_mb: number }>(
      'GET',
      '/models',
      undefined,
      30000,
    );
  }
  systemPrompt() {
    return this.request<{ content: string; default: string; path: string }>('GET', '/system-prompt');
  }
  saveSystemPrompt(content: string) {
    return this.request<{ content: string }>('PUT', '/system-prompt', { content });
  }
  async skills(): Promise<SkillInfo[]> {
    return (await this.request<{ skills: SkillInfo[] }>('GET', '/skills', undefined, 30000)).skills;
  }

  // ----- projects -----
  async projects(): Promise<Project[]> {
    return (await this.request<{ projects: Project[] }>('GET', '/projects')).projects;
  }
  /** With an empty path, the server creates a folder named after the project in the projects folder. */
  async createProject(name: string, path: string, deviceId?: string): Promise<Project> {
    const body = deviceId ? { name, path, device_id: deviceId } : { name, path };
    return (await this.request<{ project: Project }>('POST', '/projects', body, 30000)).project;
  }
  async project(id: string): Promise<Project> {
    return (await this.request<{ project: Project }>('GET', `/projects/${encodeURIComponent(id)}`)).project;
  }
  /** `model: ''` goes back to whatever model is loaded. */
  /** `device_id: ''` runs the tools on the PC again. */
  async updateProject(
    id: string,
    patch: { name?: string; instructions?: string; model?: string; device_id?: string },
  ): Promise<Project> {
    return (await this.request<{ project: Project }>('PATCH', `/projects/${encodeURIComponent(id)}`, patch)).project;
  }
  async deleteProject(id: string): Promise<void> {
    await this.request('DELETE', `/projects/${encodeURIComponent(id)}`);
  }

  // ----- devices -----
  /** Paired devices the agent can work in (phone Proot Linux, ...). */
  async devices(): Promise<Device[]> {
    return (await this.request<{ devices: Device[] }>('GET', '/devices')).devices;
  }
  /** Pairs a device host; the PC checks it answers before saving. */
  async addDevice(name: string, url: string, token: string): Promise<Device> {
    return (await this.request<{ device: Device }>('POST', '/devices', { name, url, token })).device;
  }
  async removeDevice(id: string): Promise<void> {
    await this.request('DELETE', `/devices/${encodeURIComponent(id)}`);
  }
  /** A fresh 6-digit code a new computer pairs with (valid ten minutes). */
  pairCode(): Promise<PairCode> {
    return this.request<PairCode>('POST', '/devices/pair-code', {});
  }
  renameDevice(id: string, name: string): Promise<{ device: Device }> {
    return this.request('PATCH', `/devices/${encodeURIComponent(id)}`, { name });
  }
  /** Sub-folders of a folder on a device ('' = the folder it shares). */
  browseDevice(id: string, path: string): Promise<{ path: string; root: string; folders: string[] }> {
    return this.request('GET', `/devices/${encodeURIComponent(id)}/browse?path=${encodeURIComponent(path)}`, undefined, 20000);
  }
  /** Re-checks the link and refreshes last_seen. */
  async pingDevice(id: string): Promise<{ connected: boolean; error?: string }> {
    return this.request('POST', `/devices/${encodeURIComponent(id)}/ping`, {});
  }

  // ----- sessions -----
  async sessions(projectId: string): Promise<Session[]> {
    return (await this.request<{ sessions: Session[] }>('GET', `/projects/${encodeURIComponent(projectId)}/sessions`))
      .sessions;
  }
  async createSession(projectId: string, title?: string): Promise<Session> {
    return (
      await this.request<{ session: Session }>('POST', `/projects/${encodeURIComponent(projectId)}/sessions`, {
        title,
      })
    ).session;
  }
  async session(id: string): Promise<Session> {
    return (await this.request<{ session: Session }>('GET', `/sessions/${encodeURIComponent(id)}`)).session;
  }
  async renameSession(id: string, title: string): Promise<Session> {
    return (await this.request<{ session: Session }>('PATCH', `/sessions/${encodeURIComponent(id)}`, { title }))
      .session;
  }
  /** `model: ''` makes the chat follow its project again. */
  async setSessionModel(id: string, model: string): Promise<Session> {
    return (await this.request<{ session: Session }>('PATCH', `/sessions/${encodeURIComponent(id)}`, { model }))
      .session;
  }
  /** Latest chats across all projects and plain chats. */
  async recent(limit = 30): Promise<RecentSession[]> {
    return (await this.request<{ sessions: RecentSession[] }>('GET', `/recent?limit=${limit}`)).sessions;
  }
  /** Hides a chat from the lists (true) or brings it back (false). */
  async archiveSession(id: string, archived: boolean): Promise<Session> {
    return (await this.request<{ session: Session }>('PATCH', `/sessions/${encodeURIComponent(id)}`, { archived }))
      .session;
  }
  async archiveProject(id: string, archived: boolean): Promise<Project> {
    return (await this.request<{ project: Project }>('PATCH', `/projects/${encodeURIComponent(id)}`, { archived }))
      .project;
  }
  /** Archived projects and chats. */
  async archived(): Promise<{ projects: Project[]; sessions: RecentSession[] }> {
    return this.request('GET', '/archived');
  }
  async deleteSession(id: string): Promise<void> {
    await this.request('DELETE', `/sessions/${encodeURIComponent(id)}`);
  }

  // ----- messages & tasks -----
  messages(id: string, opts: { before?: number; after?: number; limit?: number } = {}) {
    const q = new URLSearchParams();
    if (opts.before !== undefined) q.set('before', String(opts.before));
    if (opts.after !== undefined) q.set('after', String(opts.after));
    q.set('limit', String(opts.limit ?? 50));
    return this.request<{ messages: Message[]; has_more: boolean }>(
      'GET',
      `/sessions/${encodeURIComponent(id)}/messages?${q.toString()}`,
    );
  }
  sendPrompt(id: string, content: string, images: string[] = [], files: { name: string; data: string }[] = []) {
    return this.request<{ message: Message; status: string }>(
      'POST',
      `/sessions/${encodeURIComponent(id)}/prompt`,
      files.length ? { content, images, files } : { content, images },
      images.length || files.length ? 120000 : undefined,
    );
  }
  // ----- updates -----
  updates(refresh = false) {
    return this.request<UpdateInfo>('GET', `/updates${refresh ? '?refresh=true' : ''}`, undefined, 40000);
  }
  installUpdate() {
    return this.request<{ version: string; install: InstallState }>('POST', '/updates/install', {}, 60000);
  }
  /** A picture attached to a message (needs the token). */
  attachmentUrl(name: string): string {
    return `${this.creds.url}/attachments/${encodeURIComponent(name)}`;
  }
  stop(id: string) {
    return this.request<{ stopping: boolean }>('POST', `/sessions/${encodeURIComponent(id)}/stop`, {});
  }

  answerApproval(id: string, toolCallId: string, decision: ApprovalDecision, note?: string) {
    return this.request<{ ok: boolean }>('POST', `/sessions/${encodeURIComponent(id)}/approval`, {
      tool_call_id: toolCallId,
      decision,
      note,
    });
  }

  // ----- mockups -----
  /** Has the PC draw an HTML/SVG mockup to a picture (for the phone). */
  mockup(source: string, svg: boolean) {
    return this.request<{ id: string; width: number; height: number }>('POST', '/mockups', { source, svg }, 60000);
  }
  /** The picture's URL; needs the Authorization header. */
  mockupUrl(id: string): string {
    return `${this.creds.url}/mockups/${encodeURIComponent(id)}`;
  }

  // ----- push -----
  registerPushToken(token: string) {
    return this.request<{ ok: boolean; push_enabled: boolean }>('POST', '/push-tokens', { token });
  }
  removePushToken(token: string) {
    return this.request<{ ok: boolean }>('DELETE', '/push-tokens', { token });
  }
}
