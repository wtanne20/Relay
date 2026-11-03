import { router, useLocalSearchParams } from 'expo-router';
import { useEffect, useState } from 'react';
import { KeyboardAvoidingView, Platform, Pressable, ScrollView, StyleSheet, Text, View } from 'react-native';

import { colors, space } from '../components/theme';
import { Banner, Button, Card, Field } from '../components/ui';
import { DEFAULT_PORT, RelayClient, describeError, normalizeServerUrl } from '../lib/api';
import { useConnection } from '../lib/connection';
import type { ServerInfo } from '../lib/types';

export default function ConnectScreen() {
  const { creds, save } = useConnection();
  const params = useLocalSearchParams<{ edit?: string; url?: string; token?: string }>();
  const edit = params.edit;
  const [address, setAddress] = useState(params.url ?? creds?.url ?? '');
  const [token, setToken] = useState(params.token ?? creds?.token ?? '');
  const [manual, setManual] = useState(Boolean(edit || params.url || Platform.OS === 'web'));
  const [showToken, setShowToken] = useState(false);
  const [testing, setTesting] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [info, setInfo] = useState<ServerInfo | null>(null);

  useEffect(() => {
    if (creds && !address && !token) {
      setAddress(creds.url);
      setToken(creds.token);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [creds]);

  const url = normalizeServerUrl(address);
  const cleanToken = token.trim();
  const canSubmit = url.length > 0 && cleanToken.length > 0;

  const test = async (): Promise<boolean> => {
    setError(null);
    setInfo(null);
    if (!canSubmit) {
      setError('Enter the server address and token.');
      return false;
    }
    setTesting(true);
    try {
      const i = await new RelayClient({ url, token: cleanToken }).info();
      setInfo(i);
      return true;
    } catch (e) {
      setError(describeError(e));
      return false;
    } finally {
      setTesting(false);
    }
  };

  const onSave = async () => {
    setSaving(true);
    try {
      const ok = info !== null || (await test());
      if (!ok) return;
      await save({ url, token: cleanToken });
      if (edit && router.canGoBack()) router.back();
      else router.replace('/');
    } catch (e) {
      setError(describeError(e));
    } finally {
      setSaving(false);
    }
  };

  return (
    <KeyboardAvoidingView style={styles.flex} behavior={Platform.OS === 'ios' ? 'padding' : undefined}>
      <ScrollView contentContainerStyle={styles.content} keyboardShouldPersistTaps="handled">
        {!edit ? (
          <View style={styles.hero}>
            <Text style={styles.heroTitle}>Pair with your PC</Text>
            <Text style={styles.intro}>
              Install and open Relay on your PC. It shows a QR code (or open Start menu → Relay → Pair phone). Make sure
              Tailscale is on, on both devices.
            </Text>
            {Platform.OS !== 'web' ? (
              <Button
                title="Scan QR code"
                onPress={() => router.push('/scan')}
                accessibilityLabel="Scan pairing QR code"
              />
            ) : null}
            {!manual ? <Button title="Enter address manually" variant="ghost" onPress={() => setManual(true)} /> : null}
          </View>
        ) : null}
        {manual ? (
          <>
            <Text style={styles.intro}>
              Use the PC's Tailscale IP (100.x.y.z) or MagicDNS name. Port {DEFAULT_PORT} is added if you leave it out.
              The token is shown in the Relay window on the PC.
            </Text>
            <Field
              label="Server address"
              placeholder={`my-pc.tail1234.ts.net:${DEFAULT_PORT}`}
              value={address}
              onChangeText={(t) => {
                setAddress(t);
                setInfo(null);
              }}
              keyboardType="url"
              textContentType="URL"
              returnKeyType="next"
              hint={url && url !== address.trim() ? `Will connect to ${url}` : undefined}
            />
            <View>
              <Field
                label="API token"
                placeholder="Shown in the Relay window on the PC"
                value={token}
                onChangeText={(t) => {
                  setToken(t);
                  setInfo(null);
                }}
                secureTextEntry={!showToken}
                textContentType="password"
                returnKeyType="done"
                onSubmitEditing={test}
              />
              <Pressable
                onPress={() => setShowToken((s) => !s)}
                style={styles.toggle}
                hitSlop={8}
                accessibilityRole="button"
              >
                <Text style={styles.toggleText}>{showToken ? 'Hide' : 'Show'}</Text>
              </Pressable>
            </View>

            {error ? <Banner message={error} actionLabel="Retry" onAction={test} /> : null}
            {info ? (
              <Card style={styles.ok}>
                <Text style={styles.okTitle}>✓ Connected to Relay {info.version}</Text>
                <Text style={styles.okBody}>
                  {info.hostname ? `${info.hostname} · ` : ''}
                  {info.projects} project{info.projects === 1 ? '' : 's'} · model {info.model.state}
                </Text>
              </Card>
            ) : null}

            <View style={styles.actions}>
              <Button
                title="Test connection"
                variant="secondary"
                onPress={test}
                loading={testing}
                disabled={!canSubmit}
              />
              <Button title="Save" onPress={onSave} loading={saving} disabled={!canSubmit || testing} />
            </View>
          </>
        ) : null}
      </ScrollView>
    </KeyboardAvoidingView>
  );
}

const styles = StyleSheet.create({
  flex: { flex: 1, backgroundColor: colors.bg },
  content: { padding: space.lg, paddingBottom: 48 },
  intro: { color: colors.textMuted, fontSize: 15, lineHeight: 21, marginBottom: space.lg },
  hero: { gap: space.md, marginBottom: space.xl },
  heroTitle: { color: colors.text, fontSize: 24, fontWeight: '700' },
  toggle: { position: 'absolute', right: space.md, top: 34, height: 32, justifyContent: 'center' },
  toggleText: { color: colors.accent, fontWeight: '600' },
  ok: { marginVertical: space.sm, borderColor: colors.success },
  okTitle: { color: colors.success, fontSize: 16, fontWeight: '600' },
  okBody: { color: colors.textMuted, marginTop: space.xs },
  actions: { gap: space.md, marginTop: space.lg },
});
