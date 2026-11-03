import { router, useLocalSearchParams } from 'expo-router';
import { useEffect, useState } from 'react';
import { KeyboardAvoidingView, Platform, ScrollView, StyleSheet, Text, View } from 'react-native';

import { m3, radius, space, type } from '../components/theme';
import { Banner, Button, Field, Icon, IconButton, Logo, type IconName } from '../components/ui';

function Step({ icon, text }: { icon: IconName; text: string }) {
  return (
    <View style={styles.step}>
      <View style={styles.stepIcon}>
        <Icon name={icon} size={20} color={m3.onSecondaryContainer} />
      </View>
      <Text style={[type.bodyLarge, styles.grow]}>{text}</Text>
    </View>
  );
}
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
            <View style={styles.logo}>
              <Logo size={80} />
            </View>
            <Text style={[type.displaySmall, styles.center]}>Pair with your PC</Text>
            <Text style={[type.bodyLarge, styles.lead]}>
              Chat with the coding agent on your PC from anywhere, privately over Tailscale.
            </Text>
            <View style={styles.steps}>
              <Step icon="download-outline" text="Install and open Relay on your PC." />
              <Step icon="shield-lock-outline" text="Turn on Tailscale on the PC and this phone." />
              <Step icon="qrcode" text="Scan the QR code in the Relay app on your PC (Settings, Pair your phone)." />
            </View>
            {Platform.OS !== 'web' ? (
              <Button
                title="Scan QR code"
                icon="qrcode-scan"
                onPress={() => router.push('/scan')}
                accessibilityLabel="Scan pairing QR code"
                style={styles.wide}
              />
            ) : null}
            {!manual ? (
              <Button
                title="Enter address manually"
                variant="text"
                onPress={() => setManual(true)}
                style={styles.wide}
              />
            ) : null}
          </View>
        ) : null}
        {manual ? (
          <View style={styles.form}>
            {!edit ? <Text style={[type.titleMedium, styles.formTitle]}>Connect manually</Text> : null}
            <Text style={[type.bodyMedium, styles.formIntro]}>
              Use the PC&apos;s Tailscale IP (100.x.y.z) or MagicDNS name. Port {DEFAULT_PORT} is added if you leave it
              out. The token is shown in the Relay window on the PC.
            </Text>
            <Field
              label="Server address"
              icon="lan-connect"
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
                icon="key-outline"
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
                style={styles.tokenInput}
              />
              <View style={styles.toggle}>
                <IconButton
                  icon={showToken ? 'eye-off-outline' : 'eye-outline'}
                  label={showToken ? 'Hide token' : 'Show token'}
                  onPress={() => setShowToken((v) => !v)}
                />
              </View>
            </View>

            {error ? <Banner message={error} actionLabel="Retry" onAction={test} /> : null}
            {info ? (
              <View style={styles.ok}>
                <Icon name="check-circle" size={24} color={m3.success} />
                <View style={styles.grow}>
                  <Text style={[type.titleMedium, { color: m3.success }]}>Connected to Relay {info.version}</Text>
                  <Text style={type.bodyMedium}>
                    {info.hostname ? `${info.hostname} · ` : ''}
                    {info.projects} project{info.projects === 1 ? '' : 's'} · model {info.model.state}
                  </Text>
                </View>
              </View>
            ) : null}

            <View style={styles.actions}>
              <Button
                title="Test"
                variant="outlined"
                icon="lan-pending"
                onPress={test}
                loading={testing}
                disabled={!canSubmit}
                style={styles.grow}
              />
              <Button
                title="Connect"
                icon="check"
                onPress={onSave}
                loading={saving}
                disabled={!canSubmit || testing}
                style={styles.grow}
              />
            </View>
          </View>
        ) : null}
      </ScrollView>
    </KeyboardAvoidingView>
  );
}

const styles = StyleSheet.create({
  flex: { flex: 1, backgroundColor: m3.surface },
  grow: { flex: 1, minWidth: 0 },
  center: { textAlign: 'center' },
  content: { padding: space.xl, paddingBottom: 48 },
  hero: { alignItems: 'center', gap: space.md, marginBottom: space.xl },
  logo: { marginBottom: space.sm },
  lead: { color: m3.onSurfaceVariant, textAlign: 'center', maxWidth: 360 },
  steps: {
    alignSelf: 'stretch',
    backgroundColor: m3.surfaceContainerLow,
    borderRadius: radius.xl,
    padding: space.lg,
    gap: space.lg,
    marginVertical: space.lg,
  },
  step: { flexDirection: 'row', alignItems: 'center', gap: space.lg },
  stepIcon: {
    width: 40,
    height: 40,
    borderRadius: 20,
    backgroundColor: m3.secondaryContainer,
    alignItems: 'center',
    justifyContent: 'center',
  },
  wide: { alignSelf: 'stretch' },
  form: { gap: space.xs },
  formTitle: { marginBottom: space.xs },
  formIntro: { marginBottom: space.lg },
  tokenInput: { paddingRight: 44 },
  toggle: { position: 'absolute', right: space.xs, top: 26 },
  ok: {
    flexDirection: 'row',
    alignItems: 'center',
    gap: space.md,
    backgroundColor: m3.successContainer,
    borderRadius: radius.lg,
    padding: space.lg,
    marginVertical: space.sm,
  },
  actions: { flexDirection: 'row', gap: space.md, marginTop: space.lg },
});
