/**
 * Relay for Windows runs this same app inside a Tauri window. There the
 * server runs on the same PC, so the app connects to it directly instead of
 * pairing with a QR code.
 */
import type { Credentials } from './storage';

interface TauriGlobal {
  core: { invoke: <T>(cmd: string, args?: Record<string, unknown>) => Promise<T> };
}

function tauri(): TauriGlobal | null {
  const t = (globalThis as { __TAURI__?: TauriGlobal }).__TAURI__;
  return t && typeof t.core?.invoke === 'function' ? t : null;
}

/** True inside the Relay desktop app. */
export const isDesktop = tauri() !== null;

export interface LocalConnection extends Credentials {
  data_dir: string;
}

export type ServerStatus =
  | { state: 'starting' | 'running' | 'external' | 'stopped' }
  | { state: 'failed'; message: string };

async function invoke<T>(cmd: string): Promise<T> {
  const t = tauri();
  if (!t) throw new Error('Not running in the Relay desktop app');
  return t.core.invoke<T>(cmd);
}

export const desktop = {
  localConnection: () => invoke<LocalConnection>('local_connection'),
  serverStatus: () => invoke<ServerStatus>('server_status'),
  openDataFolder: () => invoke<void>('open_data_folder'),
  quit: () => invoke<void>('quit'),
};
