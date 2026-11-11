import Constants from 'expo-constants';
import { router, useFocusEffect } from 'expo-router';
import { useCallback, useEffect, useState, useSyncExternalStore } from 'react';
import {
  ActivityIndicator,
  Image,
  Platform,
  Pressable,
  RefreshControl,
  ScrollView,
  StyleSheet,
  Text,
  View,
} from 'react-native';

import { ModelStatusBar } from '../components/ModelStatusBar';
import { Group, NavRow, SectionTitle, ToggleRow } from '../components/SettingsControls';
import { ACCENTS, type AccentId, accent, m3, onAccentChange, setAccent, space, type } from '../components/theme';
import { Sheet } from '../components/Sheet';
import { UpdateCard } from '../components/UpdateCard';
import { Banner, Button, Field, Icon, type IconName } from '../components/ui';
import { alert } from '../components/Dialogs';
import { describeError } from '../lib/api';
import { useConnection } from '../lib/connection';
import { desktop, desktopPlatform, isDesktop } from '../lib/desktop';
import { usePageStyle } from '../lib/layout';
import { formatDuration } from '../lib/format';
import { disablePush, enablePush, pushSupported } from '../lib/push';
import { watchAvailable } from '../lib/watch';
import { loadPushPrefs } from '../lib/storage';
import type { ServerInfo } from '../lib/types';

/** Desktop: the QR code the phone app scans, straight from the local server. */
function PairPhone() {
  const { client } = useConnection();
  const [pairing, setPairing] = useState<{ url: string; svg: string; tailscale: boolean } | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    client
      ?.pairing()
      .then(setPairing)
      .catch((e) => setError(describeError(e)));
  }, [client]);
  if (error) return <Banner message={error} />;
  if (!pairing) return <ActivityIndicator style={styles.pad} color={m3.primary} />;
  // The SVG is plain ASCII, so btoa is safe; base64 data URIs render reliably on react-native-web.
  const uri = `data:image/svg+xml;base64,${globalThis.btoa(pairing.svg)}`;
  return (
    <View style={styles.pair}>
      <View style={styles.qr}>
        <Image source={{ uri }} style={styles.qrImage} accessibilityLabel="Pairing QR code" />
      </View>
      <View style={styles.grow}>
        <Text style={type.titleMedium}>Pair your phone</Text>
        <Text style={[type.bodyMedium, styles.pairText]}>
          1. Turn on Tailscale on this PC and your phone.{'\n'}2. Open Relay on the phone and tap Scan QR code.
          {'\n'}3. Point it at this code.
        </Text>
        <Text style={type.bodySmall}>Address: {pairing.url}</Text>
        {!pairing.tailscale ? (
          <Text style={[type.bodySmall, { color: m3.warning, marginTop: space.xs }]}>
            Tailscale was not found on this PC, so the phone can only connect on your home network.
          </Text>
        ) : null}
        <Text style={[type.bodySmall, { marginTop: space.sm }]}>
          The code contains your access token. Don&apos;t share it.
        </Text>
      </View>
    </View>
  );
}

/** Swatches for the accent colour used across the app. */
function AccentPicker() {
  const current = useSyncExternalStore(onAccentChange, accent, accent);
  return (
    <View style={styles.accentRow}>
      <View style={styles.accentText}>
        <Text style={type.bodyLarge}>Accent colour</Text>
        <Text style={type.bodySmall}>{ACCENTS[current].name}</Text>
      </View>
      <View style={styles.swatches}>
        {(Object.keys(ACCENTS) as AccentId[]).map((id) => {
          const on = id === current;
          return (
            <Pressable
              key={id}
              onPress={() => setAccent(id)}
              accessibilityRole="radio"
              accessibilityState={{ selected: on }}
              accessibilityLabel={ACCENTS[id].name}
              style={({ hovered }: { pressed: boolean; hovered?: boolean }) => [
                styles.swatchRing,
                on && { borderColor: ACCENTS[id].primary },
                hovered && !on && { borderColor: m3.outline },
              ]}
            >
              <View style={[styles.swatch, { backgroundColor: ACCENTS[id].primary }]}>
                {on ? <Icon name="check" size={18} color={ACCENTS[id].onPrimary} /> : null}
              </View>
            </Pressable>
          );
        })}
      </View>
    </View>
  );
}

/** A folder setting (projects or models) with a dialog to change it. */
function FolderSetting({
  setting,
  icon,
  label,
  empty,
  placeholder,
  hint,
}: {
  setting: 'projects_folder';
  icon: IconName;
  label: string;
  empty: string;
  placeholder: string;
  hint: string;
}) {
  const { client } = useConnection();
  const [folder, setFolder] = useState<string | null>(null);
  const [draft, setDraft] = useState('');
  const [open, setOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useFocusEffect(
    useCallback(() => {
      client
        ?.settings()
        .then((r) => setFolder(r.settings[setting] ?? ''))
        .catch(() => undefined);
    }, [client, setting]),
  );

  const save = async () => {
    if (!client) return;
    setBusy(true);
    setError(null);
    try {
      const r = await client.patchSettings({ [setting]: draft.trim() });
      setFolder(r.settings[setting] ?? '');
      setOpen(false);
    } catch (e) {
      setError(describeError(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <>
      <NavRow
        icon={icon}
        label={label}
        value={folder === null ? '…' : folder || empty}
        onPress={() => {
          setDraft(folder ?? '');
          setError(null);
          setOpen(true);
        }}
      />
      <Sheet visible={open} onClose={() => setOpen(false)} title={label}>
        <Field
          label="Folder on your PC"
          icon={icon}
          placeholder={placeholder}
          value={draft}
          onChangeText={setDraft}
          hint={hint}
          autoFocus
          onSubmitEditing={save}
        />
        {error ? <Banner message={error} /> : null}
        <View style={styles.sheetActions}>
          <Button title="Cancel" variant="text" onPress={() => setOpen(false)} />
          <Button
            title="Save"
            icon="check"
            onPress={save}
            loading={busy}
            disabled={setting === 'projects_folder' && !draft.trim()}
          />
        </View>
      </Sheet>
    </>
  );
}

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
  const page = usePageStyle(760);
  // Push notifications and pairing settings belong to the phone app.
  const phone = Platform.OS !== 'web';

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
    loadPushPrefs().then((p) => setPush(p.enabled && (p.token !== null || watchAvailable)));
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
    alert(
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
              alert('Restart failed', describeError(e));
            } finally {
              setRestarting(false);
            }
          },
        },
      ],
    );
  };

  const onDisconnect = () => {
    alert('Disconnect?', 'The saved address and token are removed from this phone.', [
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
      contentContainerStyle={[styles.content, page]}
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
      <ModelStatusBar client={client} active={focused} controls />

      <SectionTitle title="Appearance" />
      <Group>
        <AccentPicker />
      </Group>

      <SectionTitle title="Models" />
      <Group>
        <NavRow
          icon="chip"
          label="Choose model"
          value="The model Relay loads by default, and tags"
          onPress={() => router.push('/models')}
        />
        <NavRow
          icon="folder-download-outline"
          label="Model libraries"
          value="Where models are saved, disk space, move models"
          onPress={() => router.push('/libraries')}
        />
        <NavRow
          icon="download-outline"
          label="Download models"
          value="Find and download models from Hugging Face"
          onPress={() => router.push('/hub')}
        />
        <NavRow icon="restart" label={restarting ? 'Restarting model…' : 'Restart model'} onPress={restartModel} />
      </Group>

      <SectionTitle title="Agent" />
      <Group>
        <NavRow
          icon="robot-outline"
          label="Agent settings"
          value="Sampling, tools, approvals"
          onPress={() => router.push('/agent')}
        />
        <NavRow
          icon="text-box-outline"
          label="System prompt"
          value="Instructions the agent always follows"
          onPress={() => router.push('/system-prompt')}
        />
      </Group>

      <SectionTitle title="Projects" />
      <Group>
        <NavRow
          icon="laptop"
          label="Computers"
          value="The brain, and computers linked to it"
          onPress={() => router.push('/devices')}
        />
        <FolderSetting
          setting="projects_folder"
          icon="folder-home-outline"
          label="Projects folder"
          empty="Not set. New projects ask for one."
          placeholder="C:\Users\me\Projects"
          hint="New projects get a folder named after them in here. It's created if it doesn't exist. Existing projects stay where they are."
        />
      </Group>

      {phone ? (
        <>
          <SectionTitle title="Notifications" />
          <Group>
            <ToggleRow
              icon="bell-outline"
              label="Notifications on this phone"
              hint={
                watchAvailable
                  ? 'When a chat finishes or needs your approval, even with Relay closed. While a chat works, Android shows a small "working" notification. Only the project name, chat title and status are shown.'
                  : 'When a task ends or needs your approval. Only the project name, chat title and status are sent.'
              }
              value={push}
              onChange={togglePush}
              disabled={pushBusy || !pushSupported()}
            />
            {!pushSupported() ? (
              <View style={styles.note}>
                <Icon name="information-outline" size={18} color={m3.onSurfaceVariant} />
                <Text style={[type.bodyMedium, styles.grow]}>
                  Notifications aren't set up for this version of the app yet. They need the app to be registered with a
                  push service, which comes in a later update.
                </Text>
              </View>
            ) : null}
            {info && !info.push_enabled && !watchAvailable ? (
              <View style={styles.note}>
                <Icon name="alert-outline" size={18} color={m3.warning} />
                <Text style={[type.bodyMedium, styles.grow]}>
                  Push is turned off on the PC. Turn it on in Agent settings, under Notifications.
                </Text>
              </View>
            ) : null}
          </Group>
          {pushError ? <Banner message={pushError} /> : null}
        </>
      ) : null}

      {isDesktop ? (
        <>
          <SectionTitle title="Phone" />
          <Group>
            <PairPhone />
          </Group>
          <SectionTitle title="This PC" />
          <Group>
            <NavRow
              icon="folder-cog-outline"
              label="Open data folder"
              value="Settings file, logs, models and chat history"
              onPress={() => desktop.openDataFolder()}
            />
            <NavRow
              icon="power"
              label="Quit Relay"
              value="Stops the agent and the model. The phone can't connect until you start Relay again."
              onPress={() => desktop.quit()}
            />
          </Group>
        </>
      ) : (
        <>
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
        </>
      )}

      <SectionTitle title="Updates" />
      <UpdateCard client={client} />

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

      {isDesktop ? null : (
        <Button
          title="Disconnect this phone"
          icon="logout"
          variant="outlined"
          onPress={onDisconnect}
          style={styles.disconnect}
        />
      )}
      <Text style={[type.bodySmall, styles.footer]}>
        Relay {desktopPlatform ? `for ${desktopPlatform}` : 'for phone'} {Constants.expoConfig?.version ?? ''}
      </Text>
    </ScrollView>
  );
}

const styles = StyleSheet.create({
  accentRow: {
    flexDirection: 'row',
    alignItems: 'center',
    flexWrap: 'wrap',
    gap: space.md,
    paddingHorizontal: space.lg,
    paddingVertical: space.md,
  },
  accentText: { flexGrow: 1, minWidth: 120 },
  swatches: { flexDirection: 'row', gap: space.xs, flexWrap: 'wrap' },
  swatchRing: { padding: 3, borderRadius: 999, borderWidth: 2, borderColor: 'transparent' },
  swatch: { width: 32, height: 32, borderRadius: 16, alignItems: 'center', justifyContent: 'center' },
  flex: { flex: 1, backgroundColor: m3.surface },
  sheetActions: { flexDirection: 'row', justifyContent: 'flex-end', gap: space.sm, marginTop: space.sm },
  grow: { flex: 1, minWidth: 0 },
  content: { padding: space.lg, paddingBottom: 64 },
  pair: { flexDirection: 'row', gap: space.xl, padding: space.lg, alignItems: 'center', flexWrap: 'wrap' },
  qr: { backgroundColor: '#fff', borderRadius: 16, padding: space.sm },
  qrImage: { width: 184, height: 184 },
  pairText: { marginVertical: space.sm, lineHeight: 22 },
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
