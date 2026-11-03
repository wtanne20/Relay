import Constants from 'expo-constants';
import { router, useFocusEffect } from 'expo-router';
import { useCallback, useEffect, useState } from 'react';
import { ActivityIndicator, Alert, RefreshControl, ScrollView, StyleSheet, Switch, Text, View } from 'react-native';

import { ModelStatusBar } from '../components/ModelStatusBar';
import { colors, space, touch } from '../components/theme';
import { Banner, Button, Card } from '../components/ui';
import { describeError } from '../lib/api';
import { useConnection } from '../lib/connection';
import { formatDuration } from '../lib/format';
import { disablePush, enablePush } from '../lib/push';
import { loadPushPrefs } from '../lib/storage';
import type { ServerInfo } from '../lib/types';

function Row({ label, value }: { label: string; value: string }) {
  return (
    <View style={styles.row}>
      <Text style={styles.rowLabel}>{label}</Text>
      <Text style={styles.rowValue} selectable numberOfLines={2} ellipsizeMode="middle">
        {value}
      </Text>
    </View>
  );
}

export default function SettingsScreen() {
  const { client, creds, disconnect } = useConnection();
  const [info, setInfo] = useState<ServerInfo | null>(null);
  const [infoError, setInfoError] = useState<string | null>(null);
  const [refreshing, setRefreshing] = useState(false);
  const [focused, setFocused] = useState(false);
  const [push, setPush] = useState(false);
  const [pushBusy, setPushBusy] = useState(false);
  const [pushError, setPushError] = useState<string | null>(null);
  const [restarting, setRestarting] = useState(false);

  const load = useCallback(async () => {
    if (!client) return;
    try {
      setInfo(await client.info());
      setInfoError(null);
    } catch (e) {
      setInfoError(describeError(e));
    }
  }, [client]);

  useFocusEffect(
    useCallback(() => {
      setFocused(true);
      load();
      return () => setFocused(false);
    }, [load]),
  );

  useEffect(() => {
    loadPushPrefs().then((p) => setPush(p.enabled && p.token !== null));
  }, []);

  useEffect(() => {
    if (!client) router.replace('/connect');
  }, [client]);

  if (!client || !creds) return null;

  const togglePush = async (on: boolean) => {
    setPushBusy(true);
    setPushError(null);
    try {
      if (on) {
        await enablePush(client);
        setPush(true);
      } else {
        await disablePush(client);
        setPush(false);
      }
    } catch (e) {
      setPushError(describeError(e));
      setPush(false);
    } finally {
      setPushBusy(false);
    }
  };

  const restartModel = () => {
    Alert.alert('Restart the model?', 'llama-server will be stopped and started again. Running tasks wait until it is ready.', [
      { text: 'Cancel', style: 'cancel' },
      {
        text: 'Restart',
        style: 'destructive',
        onPress: async () => {
          setRestarting(true);
          try {
            await client.restartModel();
            await load();
          } catch (e) {
            Alert.alert('Restart failed', describeError(e));
          } finally {
            setRestarting(false);
          }
        },
      },
    ]);
  };

  const onDisconnect = () => {
    Alert.alert('Disconnect?', 'The saved address and token are removed from this phone.', [
      { text: 'Cancel', style: 'cancel' },
      {
        text: 'Disconnect',
        style: 'destructive',
        onPress: async () => {
          await disablePush(client).catch(() => undefined);
          await disconnect();
          router.replace('/connect');
        },
      },
    ]);
  };

  const maskedToken = `${creds.token.slice(0, 4)}…${creds.token.slice(-4)}`;

  return (
    <ScrollView
      style={styles.flex}
      contentContainerStyle={styles.content}
      refreshControl={
        <RefreshControl
          refreshing={refreshing}
          onRefresh={async () => {
            setRefreshing(true);
            await load();
            setRefreshing(false);
          }}
          tintColor={colors.textMuted}
        />
      }
    >
      <Text style={styles.section}>Server</Text>
      <Card>
        <Row label="Address" value={creds.url} />
        <Row label="Token" value={maskedToken} />
        <Button
          title="Edit address or token"
          variant="secondary"
          onPress={() => router.push({ pathname: '/connect', params: { edit: '1' } })}
          style={styles.cardButton}
        />
      </Card>

      <Text style={styles.section}>Model</Text>
      <View style={styles.modelBar}>
        <ModelStatusBar client={client} active={focused} />
      </View>
      <Button title="Restart model" variant="danger" onPress={restartModel} loading={restarting} />

      <Text style={styles.section}>Notifications</Text>
      <Card>
        <View style={styles.switchRow}>
          <View style={styles.grow}>
            <Text style={styles.switchLabel}>Notify when a task ends</Text>
            <Text style={styles.switchHint}>Only the project name, session title and status are sent.</Text>
          </View>
          {pushBusy ? (
            <ActivityIndicator color={colors.accent} />
          ) : (
            <Switch
              value={push}
              onValueChange={togglePush}
              trackColor={{ true: colors.accent, false: colors.border }}
              accessibilityLabel="Push notifications"
            />
          )}
        </View>
        {info && !info.push_enabled ? (
          <Text style={styles.warn}>Push is turned off on the server ([push] enabled = false in config.toml).</Text>
        ) : null}
      </Card>
      {pushError ? <Banner message={pushError} /> : null}

      <Text style={styles.section}>Server info</Text>
      {infoError ? <Banner message={infoError} actionLabel="Retry" onAction={load} /> : null}
      <Card>
        {info ? (
          <>
            <Row label="Relay" value={`v${info.version}`} />
            {info.hostname ? <Row label="PC" value={info.hostname} /> : null}
            <Row label="OS" value={info.os} />
            <Row label="Uptime" value={formatDuration(info.uptime_seconds)} />
            <Row label="Projects" value={String(info.projects)} />
            <Row label="Max steps" value={String(info.max_steps)} />
            <Row label="Model file" value={info.model.model_path || '(not set)'} />
            <Row label="llama-server" value={`127.0.0.1:${info.model.port}${info.model.external ? ' (external)' : ''}`} />
          </>
        ) : infoError ? (
          <Text style={styles.switchHint}>Unavailable</Text>
        ) : (
          <ActivityIndicator color={colors.accent} />
        )}
      </Card>

      <Button title="Disconnect" variant="ghost" onPress={onDisconnect} style={styles.disconnect} />
      <Text style={styles.footer}>Relay mobile {Constants.expoConfig?.version ?? ''}</Text>
    </ScrollView>
  );
}

const styles = StyleSheet.create({
  flex: { flex: 1, backgroundColor: colors.bg },
  grow: { flex: 1 },
  content: { padding: space.lg, paddingBottom: 64 },
  section: {
    color: colors.textMuted,
    fontSize: 13,
    fontWeight: '600',
    textTransform: 'uppercase',
    marginTop: space.xl,
    marginBottom: space.sm,
  },
  row: { flexDirection: 'row', justifyContent: 'space-between', paddingVertical: space.sm, gap: space.md },
  rowLabel: { color: colors.textMuted, fontSize: 15 },
  rowValue: { color: colors.text, fontSize: 15, flexShrink: 1, textAlign: 'right' },
  cardButton: { marginTop: space.md },
  modelBar: { marginHorizontal: -space.lg, marginTop: -space.md, marginBottom: space.sm },
  switchRow: { flexDirection: 'row', alignItems: 'center', gap: space.md, minHeight: touch },
  switchLabel: { color: colors.text, fontSize: 16, fontWeight: '500' },
  switchHint: { color: colors.textMuted, fontSize: 13, marginTop: 2 },
  warn: { color: colors.warning, fontSize: 13, marginTop: space.sm },
  disconnect: { marginTop: space.xl },
  footer: { color: colors.textFaint, textAlign: 'center', marginTop: space.md, fontSize: 12 },
});
