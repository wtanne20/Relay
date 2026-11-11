import * as Secure from 'expo-secure-store';
import { Platform } from 'react-native';

/**
 * The phone keeps credentials in the Keychain / Keystore. The web build
 * (development only) has no secure store, so it falls back to localStorage.
 */
const SecureStore =
  Platform.OS === 'web'
    ? {
        getItemAsync: async (k: string) => globalThis.localStorage?.getItem(k) ?? null,
        setItemAsync: async (k: string, v: string) => globalThis.localStorage?.setItem(k, v),
        deleteItemAsync: async (k: string) => globalThis.localStorage?.removeItem(k),
      }
    : Secure;

export interface Credentials {
  /** Normalized base URL, e.g. http://my-pc.tail1234.ts.net:8765 */
  url: string;
  token: string;
}

const CREDS_KEY = 'relay.credentials';
const PUSH_KEY = 'relay.push';

export interface PushPrefs {
  enabled: boolean;
  /** The Expo push token last registered with the server. */
  token: string | null;
}

export async function loadCredentials(): Promise<Credentials | null> {
  try {
    const raw = await SecureStore.getItemAsync(CREDS_KEY);
    if (!raw) return null;
    const parsed = JSON.parse(raw) as Partial<Credentials>;
    if (typeof parsed.url === 'string' && typeof parsed.token === 'string') {
      return { url: parsed.url, token: parsed.token };
    }
    return null;
  } catch {
    return null;
  }
}

export async function saveCredentials(creds: Credentials): Promise<void> {
  await SecureStore.setItemAsync(CREDS_KEY, JSON.stringify(creds));
}

export async function clearCredentials(): Promise<void> {
  await SecureStore.deleteItemAsync(CREDS_KEY);
}

export async function loadPushPrefs(): Promise<PushPrefs> {
  try {
    const raw = await SecureStore.getItemAsync(PUSH_KEY);
    if (!raw) return { enabled: true, token: null };
    const p = JSON.parse(raw) as Partial<PushPrefs>;
    return { enabled: p.enabled !== false, token: typeof p.token === 'string' ? p.token : null };
  } catch {
    return { enabled: true, token: null };
  }
}

export async function savePushPrefs(prefs: PushPrefs): Promise<void> {
  await SecureStore.setItemAsync(PUSH_KEY, JSON.stringify(prefs));
}

const LAST_TARGET_KEY = 'relay.lastTarget';

/**
 * Where the home screen's next chat goes: a project id, or the plain-chat id.
 * Kept in the same store as everything else, so the phone remembers it too
 * (the web build uses localStorage, under the key it always used).
 */
export async function loadLastTarget(): Promise<string | null> {
  try {
    return await SecureStore.getItemAsync(LAST_TARGET_KEY);
  } catch {
    return null;
  }
}

export async function saveLastTarget(id: string): Promise<void> {
  try {
    await SecureStore.setItemAsync(LAST_TARGET_KEY, id);
  } catch {
    // Not important: plain chat is picked next time.
  }
}
