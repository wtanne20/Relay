import * as Notifications from 'expo-notifications';
import { DarkTheme, Stack, ThemeProvider, router, usePathname } from 'expo-router';
import { StatusBar } from 'expo-status-bar';
import { useEffect, useRef, useState, useSyncExternalStore } from 'react';
import { Platform, StyleSheet, View } from 'react-native';
import { SafeAreaProvider } from 'react-native-safe-area-context';

import { DialogHost } from '../components/Dialogs';
import { Sidebar } from '../components/Sidebar';
import { accent, m3, onAccentChange, space } from '../components/theme';
import { IconButton } from '../components/ui';
import { useWide } from '../lib/layout';
import { ConnectionProvider, useConnection } from '../lib/connection';
import { emit } from '../lib/events';
import { answerFromNotification, configureNotificationHandler, refreshPushRegistration } from '../lib/push';
import { onShare, takeShare } from '../lib/share';

configureNotificationHandler();

const theme = {
  ...DarkTheme,
  colors: {
    ...DarkTheme.colors,
    primary: m3.primary,
    background: m3.surface,
    card: m3.surface,
    text: m3.onSurface,
    border: m3.outlineVariant,
    notification: m3.primary,
  },
};

/** Opens the session a tapped notification refers to, and keeps push registered. */
function PushBridge() {
  const { client } = useConnection();
  const response = Notifications.useLastNotificationResponse();
  const handled = useRef<string | null>(null);

  useEffect(() => {
    if (!response) return;
    const id = `${response.notification.request.identifier}:${response.actionIdentifier}`;
    if (handled.current === id) return;
    if (response.actionIdentifier !== Notifications.DEFAULT_ACTION_IDENTIFIER) {
      // Allow / Deny: answer it here, without opening the chat. Waits for
      // the connection when the app was started by the button.
      if (!client) return;
      handled.current = id;
      answerFromNotification(client, response).catch(() => undefined);
      return;
    }
    handled.current = id;
    const data = response.notification.request.content.data as { session_id?: unknown } | undefined;
    if (typeof data?.session_id === 'string') {
      router.push({ pathname: '/session/[id]', params: { id: data.session_id } });
    }
  }, [response, client]);

  useEffect(() => {
    if (client) refreshPushRegistration(client).catch(() => undefined);
  }, [client]);

  return null;
}

/**
 * Opens the share screen when another app shares something to Relay: on a
 * cold start with the share, and when one arrives while Relay is open.
 */
function ShareBridge() {
  const pathname = usePathname();
  const here = useRef(pathname);
  here.current = pathname;

  useEffect(() => {
    let alive = true;
    const check = async () => {
      const p = await takeShare();
      if (!alive || !p) return;
      if (here.current === '/share') emit('share');
      else router.push('/share');
    };
    // Give the navigator a moment to mount on a cold start.
    const t = setTimeout(check, 0);
    const off = onShare(check);
    return () => {
      alive = false;
      clearTimeout(t);
      off();
    };
  }, []);

  return null;
}

/** Dark scrollbars, no browser focus outlines, M3 selection colour (web/desktop only). */
function WebStyles() {
  useEffect(() => {
    if (Platform.OS !== 'web' || typeof document === 'undefined') return;
    const el = document.createElement('style');
    el.textContent = `
      :root { color-scheme: dark; }
      html, body { background: ${m3.surface}; }
      * { scrollbar-width: thin; scrollbar-color: ${m3.outlineVariant} transparent; }
      ::-webkit-scrollbar { width: 10px; height: 10px; }
      ::-webkit-scrollbar-thumb { background: ${m3.outlineVariant}; border-radius: 8px; border: 2px solid ${m3.surface}; }
      ::-webkit-scrollbar-track { background: transparent; }
      input, textarea { outline: none !important; }
      ::selection { background: ${m3.primaryContainer}; color: ${m3.onPrimaryContainer}; }
    `;
    document.head.appendChild(el);
    return () => el.remove();
  }, []);
  return null;
}

/** Ctrl+N: new chat, Ctrl+,: settings, Ctrl+B: toggle the sidebar (desktop). */
function useShortcuts(toggleSidebar: () => void) {
  useEffect(() => {
    if (Platform.OS !== 'web' || typeof window === 'undefined') return;
    const onKey = (e: KeyboardEvent) => {
      if (!(e.ctrlKey || e.metaKey) || e.altKey) return;
      const k = e.key.toLowerCase();
      if (k === 'n') router.replace('/');
      else if (k === ',') router.replace('/settings');
      else if (k === 'b') toggleSidebar();
      else return;
      e.preventDefault();
    };
    // Capture phase: text inputs on react-native-web stop key events from bubbling.
    window.addEventListener('keydown', onKey, true);
    return () => window.removeEventListener('keydown', onKey, true);
  }, [toggleSidebar]);
}

/** Phone: just the screens. Wide windows: sidebar + screens, Claude-app style. */
function Shell({ children }: { children: React.ReactNode }) {
  const wide = useWide();
  const { client } = useConnection();
  const [collapsed, setCollapsed] = useState(false);
  useShortcuts(() => setCollapsed((c) => !c));
  if (!wide || !client) return <>{children}</>;
  return (
    <View style={styles.shell}>
      {collapsed ? (
        <View style={styles.rail}>
          <IconButton icon="dock-left" label="Show sidebar" onPress={() => setCollapsed(false)} size={20} />
          <IconButton icon="plus" label="New chat" variant="tonal" onPress={() => router.replace('/')} size={20} />
          <View style={{ flex: 1 }} />
          <IconButton icon="cog-outline" label="Settings" onPress={() => router.replace('/settings')} size={20} />
        </View>
      ) : (
        <Sidebar onCollapse={() => setCollapsed(true)} />
      )}
      <View style={styles.main}>{children}</View>
    </View>
  );
}

export default function RootLayout() {
  // A new accent colour remounts the app so every screen picks it up.
  const accentId = useSyncExternalStore(onAccentChange, accent, accent);
  return (
    <SafeAreaProvider>
      <ConnectionProvider>
        <ThemeProvider value={theme}>
          <StatusBar style="light" />
          {Platform.OS !== 'web' ? <PushBridge /> : <WebStyles />}
          {Platform.OS === 'android' ? <ShareBridge /> : null}
          {Platform.OS === 'web' ? <DialogHost /> : null}
          <Shell key={accentId}>
            <Stack
              screenOptions={{
                headerStyle: { backgroundColor: m3.surface },
                headerShadowVisible: false,
                headerTintColor: m3.onSurface,
                headerTitleStyle: { fontSize: 20, fontWeight: '400' },
                contentStyle: { backgroundColor: m3.surface },
                headerBackButtonDisplayMode: 'minimal',
                animation: Platform.OS === 'web' ? 'none' : 'slide_from_right',
              }}
            >
              <Stack.Screen name="index" options={{ title: 'Relay', headerShown: false }} />
              <Stack.Screen name="connect" options={{ title: '' }} />
              <Stack.Screen name="project/[id]" options={{ title: 'Chats' }} />
              <Stack.Screen name="session/[id]" options={{ headerShown: false }} />
              <Stack.Screen name="settings" options={{ title: 'Settings' }} />
              <Stack.Screen name="scan" options={{ title: 'Scan QR code', headerTransparent: true }} />
              <Stack.Screen name="pair" options={{ title: '' }} />
              <Stack.Screen name="agent" options={{ title: 'Agent settings' }} />
              <Stack.Screen name="approvals" options={{ title: 'Always allowed' }} />
              <Stack.Screen name="activity" options={{ title: 'Activity' }} />
              <Stack.Screen name="models" options={{ title: 'Choose model' }} />
              <Stack.Screen name="hub" options={{ title: 'Download models' }} />
              <Stack.Screen name="system-prompt" options={{ title: 'System prompt' }} />
              <Stack.Screen name="archived" options={{ title: 'Archived' }} />
              <Stack.Screen name="devices" options={{ title: 'Computers' }} />
              <Stack.Screen name="libraries" options={{ title: 'Model libraries' }} />
              <Stack.Screen name="documents" options={{ title: 'Your documents' }} />
              <Stack.Screen name="project/instructions" options={{ title: 'Instructions' }} />
              <Stack.Screen name="share" options={{ title: 'Send to Relay' }} />
            </Stack>
          </Shell>
        </ThemeProvider>
      </ConnectionProvider>
    </SafeAreaProvider>
  );
}

const styles = StyleSheet.create({
  shell: { flex: 1, flexDirection: 'row', backgroundColor: m3.surfaceContainerLow },
  main: {
    flex: 1,
    minWidth: 0,
    backgroundColor: m3.surface,
    overflow: 'hidden',
  },
  rail: { width: 64, alignItems: 'center', gap: space.sm, paddingVertical: space.md },
});
