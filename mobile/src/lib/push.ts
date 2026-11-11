import Constants from 'expo-constants';
import * as Device from 'expo-device';
import * as Notifications from 'expo-notifications';
import { Platform } from 'react-native';

import type { RelayClient } from './api';
import { loadPushPrefs, savePushPrefs } from './storage';
import { setWatchEnabled, watchAvailable } from './watch';

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
  if (Platform.OS === 'web') return;
  // Allow and Deny buttons on "needs your approval" notifications; the
  // server tags those with this category. Answered without opening the app.
  Notifications.setNotificationCategoryAsync(APPROVAL_CATEGORY, [
    { identifier: DENY_ACTION, buttonTitle: 'Deny', options: { opensAppToForeground: false, isDestructive: true } },
    { identifier: APPROVE_ACTION, buttonTitle: 'Allow', options: { opensAppToForeground: false } },
  ]).catch(() => undefined);
}

export const APPROVAL_CATEGORY = 'approval';
export const APPROVE_ACTION = 'approve';
export const DENY_ACTION = 'deny';

/**
 * Answers an approval from a notification's Allow or Deny button. Returns
 * false when the response isn't one of those buttons.
 */
export async function answerFromNotification(
  client: RelayClient,
  response: Notifications.NotificationResponse,
): Promise<boolean> {
  const action = response.actionIdentifier;
  if (action !== APPROVE_ACTION && action !== DENY_ACTION) return false;
  const data = response.notification.request.content.data as
    | { session_id?: unknown; tool_call_id?: unknown }
    | undefined;
  if (typeof data?.session_id !== 'string' || typeof data?.tool_call_id !== 'string') return false;
  try {
    await client.answerApproval(data.session_id, data.tool_call_id, action === APPROVE_ACTION ? 'approve' : 'deny');
  } catch {
    // Already answered somewhere else, or the PC is out of reach: the chat
    // still shows the question when opened.
  }
  Notifications.dismissNotificationAsync(response.notification.request.identifier).catch(() => undefined);
  return true;
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
  if (watchAvailable) return; // nothing to register
  try {
    await enablePush(client);
  } catch {
    // Settings shows the error when the user looks; launch stays quiet.
  }
}
