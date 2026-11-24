/**
 * Notifications on Android without a push service. While a chat is working,
 * a small background service (modules/relay-watch) keeps an eye on it over
 * the same connection to the PC, with an ongoing "working" notification
 * (Android's rule for apps that keep running), and tells you when it
 * finishes, fails or needs approval. It stops by itself once nothing runs.
 */
import Constants from 'expo-constants';
import { requireOptionalNativeModule } from 'expo';
import * as Notifications from 'expo-notifications';
import { Platform } from 'react-native';

import type { RelayClient } from './api';
import { loadPushPrefs } from './storage';

type RelayWatch = {
  start(url: string, token: string, scheme: string, sessionId: string | null): boolean;
  stop(): void;
  setViewing(sessionId: string | null): void;
  /** Missing on builds from before background tasks. */
  startChecking?(url: string, token: string, scheme: string): boolean;
  stopChecking?(): void;
};

const native = Platform.OS === 'android' ? requireOptionalNativeModule<RelayWatch>('RelayWatch') : null;

/** Whether this build has an Expo project id, so real push works instead. */
function hasPushProject(): boolean {
  const extra = Constants.expoConfig?.extra as { eas?: { projectId?: string } } | undefined;
  return !!(extra?.eas?.projectId ?? Constants.easConfig?.projectId);
}

/** Whether notifications come from the background watcher on this phone. */
export const watchAvailable = !!native && !hasPushProject();

let enabled: boolean | null = null;

export function setWatchEnabled(on: boolean) {
  enabled = on;
  if (!on) {
    native?.stop();
    try {
      native?.stopChecking?.();
    } catch {
      // Not available on this build.
    }
  }
}

/**
 * Has the phone check in with the PC every 15 minutes or so, even with the
 * app closed, so scheduled and queued tasks that finish (or need approval)
 * while nobody watches still notify. Call while the app is open.
 */
export async function checkInBackground(client: RelayClient): Promise<void> {
  if (!native?.startChecking || !watchAvailable) return;
  try {
    if (enabled === null) enabled = (await loadPushPrefs()).enabled;
    if (!enabled) return;
    const perm = await Notifications.getPermissionsAsync();
    if (!perm.granted) return;
    const scheme = typeof Constants.expoConfig?.scheme === 'string' ? Constants.expoConfig.scheme : 'relay';
    native.startChecking(client.creds.url, client.creds.token, scheme);
  } catch {
    // Notifications are a nice-to-have.
  }
}

/** Starts watching (no-op if already watching); call while the app is open. */
export async function watchChat(client: RelayClient, sessionId: string | null): Promise<void> {
  if (!native || !watchAvailable) return;
  try {
    if (enabled === null) enabled = (await loadPushPrefs()).enabled;
    if (!enabled) return;
    let perm = await Notifications.getPermissionsAsync();
    // Asked once, the first time a chat works, while the app is open.
    if (!perm.granted && perm.canAskAgain && perm.status === 'undetermined') {
      perm = await Notifications.requestPermissionsAsync();
    }
    if (!perm.granted) return;
    const scheme = typeof Constants.expoConfig?.scheme === 'string' ? Constants.expoConfig.scheme : 'relay';
    native.start(client.creds.url, client.creds.token, scheme, sessionId);
  } catch {
    // Notifications are a nice-to-have; the chat works without them.
  }
}

/** The chat on screen, so it doesn't notify about itself. */
export function setViewing(sessionId: string | null) {
  try {
    native?.setViewing(sessionId);
  } catch {
    // Not available on this build.
  }
}
