import { router, useLocalSearchParams } from 'expo-router';
import { useEffect, useState } from 'react';
import { ActivityIndicator, ScrollView, StyleSheet, Text, View } from 'react-native';

import { colors, space } from '../components/theme';
import { Banner, Button, Card } from '../components/ui';
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
        <Banner message="This pairing link is incomplete. Scan the QR code on your PC again." />
        <Button title="Scan again" onPress={() => router.replace('/scan')} />
      </View>
    );
  }

  return (
    <ScrollView style={styles.flex} contentContainerStyle={styles.content}>
      <Text style={styles.address}>{creds.url}</Text>
      {info ? (
        <Card style={styles.ok}>
          <Text style={styles.okTitle}>✓ Paired with {info.hostname || 'your PC'}</Text>
          <Text style={styles.okBody}>
            Model: {info.model.state === 'ready' ? 'ready' : (info.model.message ?? info.model.state)}
          </Text>
        </Card>
      ) : error ? (
        <>
          <Banner message={error} actionLabel="Retry" onAction={() => setAttempt((a) => a + 1)} />
          <Text style={styles.tips}>
            Check that Tailscale is switched on, on this phone and on the PC, and that the Relay window is open on the
            PC.
          </Text>
          <Button title="Scan again" variant="secondary" onPress={() => router.replace('/scan')} />
          <Button
            title="Enter address manually"
            variant="ghost"
            onPress={() => router.replace({ pathname: '/connect', params: { url: creds.url, token: creds.token } })}
          />
        </>
      ) : (
        <View style={styles.row}>
          <ActivityIndicator color={colors.accent} />
          <Text style={styles.okBody}>Connecting over Tailscale…</Text>
        </View>
      )}
    </ScrollView>
  );
}

const styles = StyleSheet.create({
  flex: { flex: 1, backgroundColor: colors.bg },
  center: { flex: 1, backgroundColor: colors.bg, justifyContent: 'center', padding: space.xl, gap: space.md },
  content: { padding: space.lg, gap: space.md },
  address: { color: colors.textMuted, fontSize: 15 },
  row: { flexDirection: 'row', alignItems: 'center', gap: space.md, paddingVertical: space.lg },
  ok: { borderColor: colors.success },
  okTitle: { color: colors.success, fontSize: 18, fontWeight: '600' },
  okBody: { color: colors.textMuted, marginTop: space.xs, fontSize: 15 },
  tips: { color: colors.textMuted, fontSize: 15, lineHeight: 21 },
});
