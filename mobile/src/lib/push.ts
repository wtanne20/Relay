import Constants from 'expo-constants';
import * as Device from 'expo-device';
import * as Notifications from 'expo-notifications';
import { Platform } from 'react-native';

import type { RelayClient } from './api';
import { loadPushPrefs, savePushPrefs } from './storage';

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
  if (!pid) {
    throw new Error(
      'This build has no EAS project id. Run `npx eas-cli@latest init` in mobile/ and rebuild (see README → Push notifications).',
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
}

/** Re-registers on launch if the user has push turned on (tokens can change). */
export async function refreshPushRegistration(client: RelayClient): Promise<void> {
  const prefs = await loadPushPrefs();
  if (!prefs.enabled) return;
  const perm = await Notifications.getPermissionsAsync();
  if (!perm.granted && prefs.token === null) return; // never set up; don't prompt on launch
  try {
    await enablePush(client);
  } catch {
    // Settings shows the error when the user looks; launch stays quiet.
  }
}
