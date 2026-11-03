import * as Notifications from 'expo-notifications';
import { DarkTheme, Stack, ThemeProvider, router } from 'expo-router';
import { StatusBar } from 'expo-status-bar';
import { useEffect, useRef } from 'react';
import { Platform } from 'react-native';
import { SafeAreaProvider } from 'react-native-safe-area-context';

import { m3 } from '../components/theme';
import { ConnectionProvider, useConnection } from '../lib/connection';
import { configureNotificationHandler, refreshPushRegistration } from '../lib/push';

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
    const id = response.notification.request.identifier;
    if (handled.current === id) return;
    handled.current = id;
    const data = response.notification.request.content.data as { session_id?: unknown } | undefined;
    if (typeof data?.session_id === 'string') {
      router.push({ pathname: '/session/[id]', params: { id: data.session_id } });
    }
  }, [response]);

  useEffect(() => {
    if (client) refreshPushRegistration(client).catch(() => undefined);
  }, [client]);

  return null;
}

export default function RootLayout() {
  return (
    <SafeAreaProvider>
      <ConnectionProvider>
        <ThemeProvider value={theme}>
          <StatusBar style="light" />
          {Platform.OS !== 'web' ? <PushBridge /> : null}
          <Stack
            screenOptions={{
              headerStyle: { backgroundColor: m3.surface },
              headerShadowVisible: false,
              headerTintColor: m3.onSurface,
              headerTitleStyle: { fontSize: 20, fontWeight: '400' },
              contentStyle: { backgroundColor: m3.surface },
              headerBackButtonDisplayMode: 'minimal',
              animation: 'slide_from_right',
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
            <Stack.Screen name="models" options={{ title: 'Choose model' }} />
            <Stack.Screen name="system-prompt" options={{ title: 'System prompt' }} />
            <Stack.Screen name="project/instructions" options={{ title: 'Instructions' }} />
          </Stack>
        </ThemeProvider>
      </ConnectionProvider>
    </SafeAreaProvider>
  );
}
