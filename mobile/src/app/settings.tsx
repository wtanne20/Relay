import Constants from 'expo-constants';
import { router, useFocusEffect } from 'expo-router';
import { useCallback, useEffect, useState } from 'react';
import { ActivityIndicator, Alert, RefreshControl, ScrollView, StyleSheet, Text, View } from 'react-native';

import { ModelStatusBar } from '../components/ModelStatusBar';
import { Group, NavRow, SectionTitle, ToggleRow } from '../components/SettingsControls';
import { m3, space, type } from '../components/theme';
import { Banner, Button, Icon, type IconName } from '../components/ui';
import { describeError } from '../lib/api';
import { useConnection } from '../lib/connection';
import { formatDuration } from '../lib/format';
import { disablePush, enablePush } from '../lib/push';
import { loadPushPrefs } from '../lib/storage';
import type { ServerInfo } from '../lib/types';

function InfoRow({ icon, label, value }: { icon: IconName; label: string; value: string }) {
  return (
    <View style={styles.row}>
      <View style={styles.leading}>
        <Icon name={icon} size={22} />
      </View>
      <View style={styles.grow}>
        <Text style={type.bodyLarge}>{label}</Text>
        <Text style={type.bodyMedium} selectable numberOfLines={2} ellipsizeMode="middle">
          {value}
        </Text>
      </View>
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
    Alert.alert(
      'Restart the model?',
      'llama-server will be stopped and started again. Running tasks wait until it is ready.',
      [
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
      ],
    );
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

  const maskedToken = `${creds.token.slice(0, 4)}••••${creds.token.slice(-4)}`;
  const gpu =
    info?.gpus && info.gpus.length
      ? info.gpus.map((g) => `${g.name} · ${Math.round(g.vram_mb / 1024)} GB`).join(', ')
      : 'None detected';

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
          tintColor={m3.onSurfaceVariant}
        />
      }
    >
      <ModelStatusBar client={client} active={focused} />

      <SectionTitle title="Agent" />
      <Group>
        <NavRow
          icon="robot-outline"
          label="Agent settings"
          value="Model, sampling, tools, approvals"
          onPress={() => router.push('/agent')}
        />
        <NavRow
          icon="text-box-outline"
          label="System prompt"
          value="Instructions the agent always follows"
          onPress={() => router.push('/system-prompt')}
        />
        <NavRow icon="restart" label={restarting ? 'Restarting model…' : 'Restart model'} onPress={restartModel} />
      </Group>

      <SectionTitle title="Notifications" />
      <Group>
        <ToggleRow
          icon="bell-outline"
          label="Notifications on this phone"
          hint="When a task ends or needs your approval. Only the project name, chat title and status are sent."
          value={push}
          onChange={togglePush}
          disabled={pushBusy}
        />
        {info && !info.push_enabled ? (
          <View style={styles.note}>
            <Icon name="alert-outline" size={18} color={m3.warning} />
            <Text style={[type.bodyMedium, styles.grow]}>
              Push is turned off on the PC. Turn it on in Agent settings, under Notifications.
            </Text>
          </View>
        ) : null}
      </Group>
      {pushError ? <Banner message={pushError} /> : null}

      <SectionTitle title="Connection" />
      <Group>
        <InfoRow icon="lan-connect" label="Address" value={creds.url} />
        <InfoRow icon="key-outline" label="Token" value={maskedToken} />
        <NavRow
          icon="qrcode-scan"
          label="Pair again"
          value="Scan the QR code on your PC"
          onPress={() => router.push('/scan')}
        />
        <NavRow
          icon="pencil-outline"
          label="Edit address or token"
          onPress={() => router.push({ pathname: '/connect', params: { edit: '1' } })}
        />
      </Group>

      <SectionTitle title="Your PC" />
      {infoError ? <Banner message={infoError} actionLabel="Retry" onAction={load} /> : null}
      <Group>
        {info ? (
          <>
            <InfoRow
              icon="monitor"
              label={info.hostname ?? 'PC'}
              value={`${info.os} · up ${formatDuration(info.uptime_seconds)}`}
            />
            <InfoRow icon="expansion-card-variant" label="GPU" value={gpu} />
            <InfoRow
              icon="shield-lock-outline"
              label="Tailscale"
              value={info.tailscale ? (info.tailscale.dns_name ?? info.tailscale.ip ?? 'On') : 'Not detected on the PC'}
            />
            <InfoRow icon="cube-outline" label="Model file" value={info.model.model_path || 'Not set'} />
            <InfoRow
              icon="server-network"
              label="llama-server"
              value={`127.0.0.1:${info.model.port}${info.model.external ? ' (external)' : ''}`}
            />
            <InfoRow
              icon="information-outline"
              label="Relay"
              value={`v${info.version} · ${info.projects} project${info.projects === 1 ? '' : 's'}`}
            />
          </>
        ) : infoError ? (
          <Text style={[type.bodyMedium, styles.pad]}>Unavailable</Text>
        ) : (
          <ActivityIndicator style={styles.pad} color={m3.primary} />
        )}
      </Group>

      <Button
        title="Disconnect this phone"
        icon="logout"
        variant="outlined"
        onPress={onDisconnect}
        style={styles.disconnect}
      />
      <Text style={[type.bodySmall, styles.footer]}>Relay for phone {Constants.expoConfig?.version ?? ''}</Text>
    </ScrollView>
  );
}

const styles = StyleSheet.create({
  flex: { flex: 1, backgroundColor: m3.surface },
  grow: { flex: 1, minWidth: 0 },
  content: { padding: space.lg, paddingBottom: 64 },
  row: {
    flexDirection: 'row',
    alignItems: 'center',
    gap: space.lg,
    minHeight: 64,
    paddingHorizontal: space.lg,
    paddingVertical: space.sm,
  },
  leading: { width: 24, alignItems: 'center' },
  note: { flexDirection: 'row', gap: space.md, paddingHorizontal: space.lg, paddingBottom: space.md, paddingLeft: 56 },
  pad: { padding: space.lg },
  disconnect: { marginTop: space.xxl },
  footer: { textAlign: 'center', marginTop: space.lg },
});
