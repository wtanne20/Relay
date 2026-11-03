import { router, useLocalSearchParams } from 'expo-router';
import { useEffect, useState } from 'react';
import { ActivityIndicator, ScrollView, StyleSheet, Text, View } from 'react-native';

import { m3, radius, space, type } from '../components/theme';
import { Banner, Button, Icon } from '../components/ui';
import { RelayClient, describeError } from '../lib/api';
import { useConnection } from '../lib/connection';
import { credentialsFrom } from '../lib/pairing';
import type { ServerInfo } from '../lib/types';

/**
 * Finishes pairing: reached from the QR scanner, or directly when the phone's
 * own camera app opens a relay://pair?url=…&token=… link.
 */
export default function PairScreen() {
  const params = useLocalSearchParams<{ url?: string; token?: string }>();
  const { save } = useConnection();
  const creds = credentialsFrom(params.url, params.token);
  const [info, setInfo] = useState<ServerInfo | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [attempt, setAttempt] = useState(0);

  useEffect(() => {
    if (!creds) return;
    let alive = true;
    setError(null);
    (async () => {
      try {
        const i = await new RelayClient(creds).info();
        if (!alive) return;
        await save(creds);
        setInfo(i);
        setTimeout(() => router.replace('/'), 900);
      } catch (e) {
        if (alive) setError(describeError(e));
      }
    })();
    return () => {
      alive = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [params.url, params.token, attempt]);

  if (!creds) {
    return (
      <View style={styles.center}>
        <Icon name="link-variant-off" size={48} color={m3.error} />
        <Text style={[type.headlineMedium, styles.text]}>Link incomplete</Text>
        <Text style={[type.bodyLarge, styles.sub]}>
          This pairing link is missing its address or token. Scan the QR code on your PC again.
        </Text>
        <Button title="Scan again" icon="qrcode-scan" onPress={() => router.replace('/scan')} style={styles.wide} />
      </View>
    );
  }

  return (
    <ScrollView style={styles.flex} contentContainerStyle={styles.content}>
      {info ? (
        <>
          <View style={[styles.badge, { backgroundColor: m3.successContainer }]}>
            <Icon name="check" size={40} color={m3.success} />
          </View>
          <Text style={[type.headlineMedium, styles.text]}>Paired with {info.hostname || 'your PC'}</Text>
          <Text style={[type.bodyLarge, styles.sub]}>
            Model {info.model.state === 'ready' ? 'is ready' : `: ${info.model.message ?? info.model.state}`}
          </Text>
        </>
      ) : error ? (
        <>
          <View style={[styles.badge, { backgroundColor: m3.errorContainer }]}>
            <Icon name="lan-disconnect" size={36} color={m3.onErrorContainer} />
          </View>
          <Text style={[type.headlineMedium, styles.text]}>Couldn&apos;t reach your PC</Text>
          <Text style={[type.bodyMedium, styles.text]}>{creds.url}</Text>
          <Banner message={error} actionLabel="Retry" onAction={() => setAttempt((a) => a + 1)} />
          <Text style={[type.bodyLarge, styles.sub]}>
            Check that Tailscale is on, on this phone and on the PC, and that the Relay window is open on the PC.
          </Text>
          <Button
            title="Scan again"
            icon="qrcode-scan"
            variant="tonal"
            onPress={() => router.replace('/scan')}
            style={styles.wide}
          />
          <Button
            title="Enter address manually"
            variant="text"
            onPress={() => router.replace({ pathname: '/connect', params: { url: creds.url, token: creds.token } })}
            style={styles.wide}
          />
        </>
      ) : (
        <>
          <View style={[styles.badge, { backgroundColor: m3.primaryContainer }]}>
            <ActivityIndicator size="large" color={m3.onPrimaryContainer} />
          </View>
          <Text style={[type.headlineMedium, styles.text]}>Connecting…</Text>
          <Text style={[type.bodyLarge, styles.sub]}>Reaching {creds.url} over Tailscale</Text>
        </>
      )}
    </ScrollView>
  );
}

const styles = StyleSheet.create({
  flex: { flex: 1, backgroundColor: m3.surface },
  center: {
    flex: 1,
    backgroundColor: m3.surface,
    justifyContent: 'center',
    alignItems: 'center',
    padding: space.xl,
    gap: space.md,
  },
  content: { flexGrow: 1, justifyContent: 'center', alignItems: 'center', padding: space.xl, gap: space.md },
  badge: {
    width: 88,
    height: 88,
    borderRadius: radius.xl,
    alignItems: 'center',
    justifyContent: 'center',
    marginBottom: space.sm,
  },
  text: { textAlign: 'center' },
  sub: { textAlign: 'center', color: m3.onSurfaceVariant, maxWidth: 360 },
  wide: { alignSelf: 'stretch' },
});
