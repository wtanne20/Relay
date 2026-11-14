/**
 * The Relay desktop app (Windows and Linux) runs this same app inside a
 * Tauri window. The computer is either the brain (it runs the model and the
 * server, and the window connects to it directly) or a linked computer (it
 * shares its files with a brain elsewhere on the tailnet, and the window
 * connects to that brain).
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

/** The desktop app's platform, for labels ("Relay for Linux"); null outside the desktop app. */
export const desktopPlatform: 'Windows' | 'macOS' | 'Linux' | null = (() => {
  if (!isDesktop) return null;
  const ua = typeof navigator === 'undefined' ? '' : navigator.userAgent;
  if (/Windows/i.test(ua)) return 'Windows';
  if (/Mac OS X|Macintosh/i.test(ua)) return 'macOS';
  return 'Linux';
})();

/** The desktop app's operating system, for messages ("Windows may ask for permission"). */
export const desktopOS: 'Windows' | 'macOS' | 'Linux' = desktopPlatform ?? 'Windows';

export interface LocalConnection extends Credentials {
  data_dir: string;
}

export type ServerStatus =
  | { state: 'starting' | 'running' | 'external' | 'stopped' }
  | { state: 'failed'; message: string };

/** This device's link to its brain (linked computers only). */
export type LinkState =
  | { state: 'connecting' | 'connected' | 'revoked' }
  | { state: 'offline'; message: string };

/** What this computer is. */
export type AppMode =
  /** First launch: not set up yet. */
  | { mode: 'setup'; hostname: string; home: string }
  /** This computer is the brain. */
  | { mode: 'brain'; hostname: string }
  /** This computer is linked to a brain elsewhere. */
  | {
      mode: 'device';
      hostname: string;
      device_id: string;
      name: string;
      brain_name: string;
      brain_url: string;
      root: string;
      link: LinkState;
    };

/** A brain found on the tailnet. */
export interface FoundBrain {
  name: string;
  url: string;
  version: string;
  os: string;
}

async function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  const t = tauri();
  if (!t) throw new Error('Not running in the Relay desktop app');
  return t.core.invoke<T>(cmd, args);
}

export const desktop = {
  localConnection: () => invoke<LocalConnection>('local_connection'),
  appMode: () => invoke<AppMode>('app_mode'),
  /** Makes this computer the brain (first launch). */
  becomeBrain: () => invoke<void>('become_brain'),
  /** Brains on the tailnet that answer Relay's hello. */
  findBrains: () => invoke<FoundBrain[]>('find_brains'),
  /** Pairs with a brain using its 6-digit code and stays linked. */
  linkToBrain: (brain: string, code: string, root?: string) =>
    invoke<AppMode>('link_to_brain', { brain, code, root: root ?? null }),
  /** Forgets the brain and goes back to the first-launch choice. */
  unlink: () => invoke<void>('unlink'),
  serverStatus: () => invoke<ServerStatus>('server_status'),
  openDataFolder: () => invoke<void>('open_data_folder'),
  quit: () => invoke<void>('quit'),
};
