import Constants from 'expo-constants';
import * as Device from 'expo-device';
import * as Notifications from 'expo-notifications';
import { Platform } from 'react-native';

import type { RelayClient } from './api';
import { loadPushPrefs, savePushPrefs } from './storage';
import { checkInBackground, setWatchEnabled, watchAvailable } from './watch';

/** Show notifications while the app is open too. */
export function configureNotificationHandler() {
  Notifications.setNotificationHandler({
    handleNotification: async () => ({
      shouldShowBanner: true,
      shouldShowList: true,
      shouldPlaySound: true,
      shouldSetBadge: false,
    }),
  });
}

/**
 * Whether this phone can show task notifications: real push (with an Expo
 * project id) or the background watcher.
 */
export function pushSupported(): boolean {
  return !!projectId() || watchAvailable;
}

function projectId(): string | undefined {
  const extra = Constants.expoConfig?.extra as { eas?: { projectId?: string } } | undefined;
  return extra?.eas?.projectId ?? Constants.easConfig?.projectId ?? undefined;
}

/**
 * Asks for permission, gets this device's Expo push token and registers it
 * with Relay. Throws an Error with a user-facing message on failure.
 */
export async function enablePush(client: RelayClient): Promise<string> {
  if (!Device.isDevice) {
    throw new Error('Push notifications need a physical phone (not a simulator).');
  }
  if (Platform.OS === 'android') {
    await Notifications.setNotificationChannelAsync('tasks', {
      name: 'Task updates',
      importance: Notifications.AndroidImportance.HIGH,
    });
  }
  const current = await Notifications.getPermissionsAsync();
  let granted = current.granted;
  if (!granted) {
    const asked = await Notifications.requestPermissionsAsync();
    granted = asked.granted;
  }
  if (!granted) {
    throw new Error('Notifications are turned off for Relay in your phone settings.');
  }
  const pid = projectId();
  if (!pid && watchAvailable) {
    // No push service: the phone watches running chats itself.
    await savePushPrefs({ enabled: true, token: null });
    setWatchEnabled(true);
    await checkInBackground(client);
    return 'local';
  }
  if (!pid) {
    throw new Error(
      "Notifications aren't set up for this version of the app yet.",
    );
  }
  let token: string;
  try {
    token = (await Notifications.getExpoPushTokenAsync({ projectId: pid })).data;
  } catch (e) {
    const msg = e instanceof Error ? e.message : String(e);
    throw new Error(
      `Could not get a push token (${msg}). Expo Go on Android doesn't support push; use a development or EAS build.`,
    );
  }
  await client.registerPushToken(token);
  await savePushPrefs({ enabled: true, token });
  return token;
}

export async function disablePush(client: RelayClient | null): Promise<void> {
  const prefs = await loadPushPrefs();
  if (prefs.token && client) {
    try {
      await client.removePushToken(prefs.token);
    } catch {
      // The server may be unreachable; the preference still turns off here.
    }
  }
  await savePushPrefs({ enabled: false, token: prefs.token });
  setWatchEnabled(false);
}

/** Re-registers on launch if the user has push turned on (tokens can change). */
export async function refreshPushRegistration(client: RelayClient): Promise<void> {
  const prefs = await loadPushPrefs();
  if (!prefs.enabled) return;
  const perm = await Notifications.getPermissionsAsync();
  if (!perm.granted && prefs.token === null) return; // never set up; don't prompt on launch
  if (watchAvailable) {
    // Nothing to register; check in now and then for background tasks.
    await checkInBackground(client);
    return;
  }
  try {
    await enablePush(client);
  } catch {
    // Settings shows the error when the user looks; launch stays quiet.
  }
}
