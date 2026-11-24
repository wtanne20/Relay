import { useCallback, useEffect, useState } from 'react';
import { ActivityIndicator, FlatList, RefreshControl, StyleSheet, Text, View } from 'react-native';

import { alert } from '../components/Dialogs';
import { BrainBanner, useHere } from '../components/Location';
import { Group } from '../components/SettingsControls';
import { Sheet } from '../components/Sheet';
import { m3, radius, space, type } from '../components/theme';
import { Banner, Button, Field, Icon, IconButton } from '../components/ui';
import { describeError } from '../lib/api';
import { useClient, useConnection } from '../lib/connection';
import { desktop, isDesktop } from '../lib/desktop';
import { usePageStyle } from '../lib/layout';
import type { Device, PairCode } from '../lib/types';

/**
 * The computers around the brain: the brain itself, and every computer
 * linked to it (their folders are where device projects live).
 */
export default function DevicesScreen() {
  const page = usePageStyle();
  const client = useClient();
  const { mode, reload } = useConnection();
  const here = useHere();
  const [devices, setDevices] = useState<Device[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [refreshing, setRefreshing] = useState(false);
  const [pairing, setPairing] = useState(false);
  const [direct, setDirect] = useState(false);
  const [renaming, setRenaming] = useState<Device | null>(null);

  const load = useCallback(async () => {
    if (!client) return;
    try {
      setDevices(await client.devices());
      setError(null);
    } catch (e) {
      setError(describeError(e));
    }
  }, [client]);

  useEffect(() => {
    load();
    const t = setInterval(load, 5000);
    return () => clearInterval(t);
  }, [load]);

  if (!client) return null;

  const remove = (d: Device) => {
    alert(
      `Remove ${d.name}?`,
      'The brain forgets it and its key stops working. Its files are not touched. Projects on it stay in the list but can’t run until it is paired again.',
      [
        { text: 'Cancel', style: 'cancel' },
        {
          text: 'Remove',
          style: 'destructive',
          onPress: async () => {
            try {
              await client.removeDevice(d.id);
              setDevices((list) => list?.filter((x) => x.id !== d.id) ?? null);
            } catch (e) {
              alert('Could not remove', describeError(e));
            }
          },
        },
      ],
    );
  };

  const unlinkHere = () => {
    alert(
      'Unlink this computer?',
      'This computer stops sharing its files with the brain, and Relay asks again what this computer should be. Remove it from the brain’s list too.',
      [
        { text: 'Cancel', style: 'cancel' },
        {
          text: 'Unlink',
          style: 'destructive',
          onPress: async () => {
            await desktop.unlink().catch(() => undefined);
            reload();
          },
        },
      ],
    );
  };

  return (
    <View style={styles.flex}>
      <FlatList
        style={styles.flex}
        contentContainerStyle={[styles.content, page]}
        data={devices ?? []}
        keyExtractor={(d) => d.id}
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
        ListHeaderComponent={
          <>
            <BrainBanner />
            {isDesktop && mode?.mode === 'device' ? (
              <View style={styles.hereActions}>
                <Button title="Unlink this computer" variant="text" icon="link-variant-off" onPress={unlinkHere} />
              </View>
            ) : null}
            {error ? <Banner message={error} actionLabel="Retry" onAction={load} /> : null}
            <Text style={[type.titleSmall, styles.section]}>Linked computers</Text>
            <Text style={[type.bodyMedium, styles.intro]}>
              A linked computer shares a folder with the brain. Projects on it keep their files there, and the
              agent reads, edits and runs commands on that computer while the brain does the thinking. They connect
              over Tailscale.
            </Text>
          </>
        }
        ListEmptyComponent={
          devices ? (
            <View style={styles.empty}>
              <Text style={type.bodyMedium}>No computers linked yet.</Text>
            </View>
          ) : (
            <ActivityIndicator color={m3.primary} style={{ marginTop: space.xl }} />
          )
        }
        renderItem={({ item }) => (
          <Group>
            <View style={styles.deviceRow}>
              <View style={styles.deviceIcon}>
                <Icon name={item.os === 'android' ? 'cellphone' : 'laptop'} size={22} />
              </View>
              <View style={styles.grow}>
                <View style={styles.nameRow}>
                  <Text style={[type.bodyLarge, styles.name]} numberOfLines={1}>
                    {item.name}
                    {item.id === here.deviceId ? ' (this computer)' : ''}
                  </Text>
                  <View
                    style={[styles.dot, { backgroundColor: item.connected ? m3.success : m3.outline }]}
                    accessibilityLabel={item.connected ? 'connected' : 'offline'}
                  />
                  <Text style={[type.labelMedium, { color: item.connected ? m3.success : m3.outline }]}>
                    {item.connected ? 'Connected' : 'Offline'}
                  </Text>
                </View>
                <Text style={type.bodySmall} numberOfLines={1} ellipsizeMode="middle">
                  Shares {item.root || '?'}
                </Text>
                <Text style={[type.bodySmall, styles.meta]} numberOfLines={1}>
                  {[item.hostname && item.hostname !== item.name ? item.hostname : '', item.os, item.arch, item.address]
                    .filter(Boolean)
                    .join(' · ') || (item.connection === 'direct' ? item.url : '')}
                </Text>
              </View>
              <IconButton icon="pencil-outline" label={`Rename ${item.name}`} onPress={() => setRenaming(item)} />
              <IconButton icon="trash-can-outline" label={`Remove ${item.name}`} onPress={() => remove(item)} />
            </View>
          </Group>
        )}
        ListFooterComponent={
          <View style={styles.footer}>
            <Button title="Link a computer" icon="plus" onPress={() => setPairing(true)} />
            <Button title="Add by address (advanced)" variant="text" onPress={() => setDirect(true)} />
          </View>
        }
      />
      <PairSheet visible={pairing} onClose={() => setPairing(false)} onLinked={load} devices={devices ?? []} />
      <DirectSheet
        visible={direct}
        onClose={() => setDirect(false)}
        onSaved={async (d) => {
          setDirect(false);
          setDevices((list) => [d, ...(list ?? [])]);
        }}
      />
      <RenameSheet
        device={renaming}
        onClose={() => setRenaming(null)}
        onSave={async (name) => {
          if (!renaming) return;
          await client.renameDevice(renaming.id, name);
          setRenaming(null);
          await load();
        }}
      />
    </View>
  );
}

/** Shows a 6-digit code to type on the new computer, and waits for it. */
function PairSheet({
  visible,
  onClose,
  onLinked,
  devices,
}: {
  visible: boolean;
  onClose: () => void;
  onLinked: () => void;
  devices: Device[];
}) {
  const client = useClient();
  const [code, setCode] = useState<PairCode | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [known] = useState(() => new Set<string>());
  const [linked, setLinked] = useState<Device | null>(null);

  const fresh = useCallback(async () => {
    if (!client) return;
    setError(null);
    try {
      setCode(await client.pairCode());
    } catch (e) {
      setError(describeError(e));
    }
  }, [client]);

  useEffect(() => {
    if (!visible) return;
    // Computers already connected when the code was shown don't count.
    known.clear();
    devices.filter((d) => d.connected).forEach((d) => known.add(d.id));
    setLinked(null);
    fresh();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [visible]);

  // Notice the new computer arriving.
  useEffect(() => {
    if (!visible || !client || linked) return;
    const t = setInterval(async () => {
      const list = await client.devices().catch(() => null);
      const added = list?.find((d) => d.connected && !known.has(d.id));
      if (added) {
        setLinked(added);
        onLinked();
      }
    }, 2000);
    return () => clearInterval(t);
  }, [visible, client, linked, known, onLinked]);

  const digits = code?.code ? `${code.code.slice(0, 3)} ${code.code.slice(3)}` : '··· ···';
  return (
    <Sheet visible={visible} onClose={onClose} title="Link a computer">
      {linked ? (
        <>
          <View style={styles.done}>
            <Icon name="check-circle" size={40} color={m3.success} />
            <Text style={type.titleMedium}>{linked.name} is linked</Text>
            <Text style={[type.bodyMedium, styles.center]}>
              New projects can now use its folders. Choose it under “Where the files are” when you add a project.
            </Text>
          </View>
          <View style={styles.sheetActions}>
            <Button title="Done" icon="check" onPress={onClose} />
          </View>
        </>
      ) : (
        <>
          <Text style={[type.bodyMedium, styles.hint]}>
            On the other computer, install Relay and open it. Choose{' '}
            <Text style={styles.strong}>Use another computer as the brain</Text>, pick this brain
            {code?.brain_name ? ` (${code.brain_name})` : ''} from the list, and type this code:
          </Text>
          <View style={styles.codeBox}>
            <Text style={styles.code} selectable>
              {digits}
            </Text>
          </View>
          <Text style={[type.bodySmall, styles.center]}>
            Works once, for ten minutes. Both computers need Tailscale, signed in to the same account.
          </Text>
          {code ? (
            <Text style={[type.bodySmall, styles.center, styles.meta]} selectable>
              Brain address, if it isn’t listed: {code.brain_url}
            </Text>
          ) : null}
          {code && !code.tailscale ? (
            <Banner message="Tailscale isn't running on the brain. Install it and sign in, or other computers can't reach it." />
          ) : null}
          {error ? <Banner message={error} /> : null}
          <View style={styles.waiting}>
            <ActivityIndicator size="small" color={m3.primary} />
            <Text style={type.bodySmall}>Waiting for the computer to connect…</Text>
          </View>
          <Text style={[type.bodySmall, styles.meta]}>
            Phone or terminal only? Run{' '}
            <Text style={styles.mono}>relay link --brain {code?.brain_url ?? '<address>'} --code {code?.code ?? '<code>'}</Text>
          </Text>
          <View style={styles.sheetActions}>
            <Button title="New code" variant="text" icon="refresh" onPress={fresh} />
            <Button title="Close" variant="text" onPress={onClose} />
          </View>
        </>
      )}
    </Sheet>
  );
}

function RenameSheet({
  device,
  onClose,
  onSave,
}: {
  device: Device | null;
  onClose: () => void;
  onSave: (name: string) => Promise<void>;
}) {
  const [name, setName] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    setName(device?.name ?? '');
    setError(null);
  }, [device]);
  return (
    <Sheet visible={!!device} onClose={onClose} title="Rename computer">
      <Field label="Name" icon="laptop" value={name} onChangeText={setName} autoFocus />
      {error ? <Banner message={error} /> : null}
      <View style={styles.sheetActions}>
        <Button title="Cancel" variant="text" onPress={onClose} />
        <Button
          title="Save"
          icon="check"
          loading={busy}
          disabled={!name.trim()}
          onPress={async () => {
            setBusy(true);
            try {
              await onSave(name.trim());
            } catch (e) {
              setError(describeError(e));
            } finally {
              setBusy(false);
            }
          }}
        />
      </View>
    </Sheet>
  );
}

/** The older way: the brain calls a `relay host` at an address. */
function DirectSheet({
  visible,
  onClose,
  onSaved,
}: {
  visible: boolean;
  onClose: () => void;
  onSaved: (d: Device) => void;
}) {
  const client = useClient();
  const [name, setName] = useState('');
  const [url, setUrl] = useState('');
  const [token, setToken] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (visible) {
      setName('');
      setUrl('');
      setToken('');
      setError(null);
    }
  }, [visible]);

  const save = async () => {
    if (!client) return;
    setBusy(true);
    setError(null);
    try {
      const d = await client.addDevice(name.trim() || 'this-device', url.trim(), token.trim());
      onSaved(d);
    } catch (e) {
      setError(describeError(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Sheet visible={visible} onClose={onClose} title="Add by address">
      <Text style={[type.bodySmall, styles.hint]}>
        For a machine running <Text style={styles.mono}>relay host</Text>, which the brain connects to. Linking with a
        code is easier and needs no open port; use this only if you need it.
      </Text>
      <Field label="Name" icon="laptop" placeholder="phone" value={name} onChangeText={setName} />
      <Field
        label="Address"
        icon="lan-connect"
        placeholder="http://100.64.0.5:8766"
        hint="Its Tailscale address and the host port (default 8766)."
        value={url}
        onChangeText={setUrl}
      />
      <Field label="Token" icon="key-outline" placeholder="the token relay host printed" value={token} onChangeText={setToken} />
      {error ? <Banner message={error} /> : null}
      <View style={styles.sheetActions}>
        <Button title="Cancel" variant="text" onPress={onClose} />
        <Button title="Add" icon="check" onPress={save} loading={busy} disabled={!url.trim() || !token.trim()} />
      </View>
    </Sheet>
  );
}

const styles = StyleSheet.create({
  flex: { flex: 1, backgroundColor: m3.surface },
  content: { padding: space.lg, paddingBottom: 96 },
  section: { marginTop: space.xl, marginBottom: space.xs, marginHorizontal: space.xs },
  intro: { marginBottom: space.md, marginHorizontal: space.xs, lineHeight: 22, color: m3.onSurfaceVariant },
  hereActions: { flexDirection: 'row', justifyContent: 'flex-end', marginTop: space.xs },
  empty: { paddingVertical: space.xl, alignItems: 'center' },
  deviceRow: {
    flexDirection: 'row',
    alignItems: 'center',
    gap: space.sm,
    paddingHorizontal: space.lg,
    paddingVertical: space.sm,
  },
  deviceIcon: {
    width: 44,
    height: 44,
    borderRadius: radius.md,
    backgroundColor: m3.surfaceContainerHighest,
    alignItems: 'center',
    justifyContent: 'center',
  },
  grow: { flex: 1, minWidth: 0 },
  nameRow: { flexDirection: 'row', alignItems: 'center', gap: space.xs },
  name: { flexShrink: 1 },
  meta: { color: m3.outline },
  dot: { width: 8, height: 8, borderRadius: 4 },
  footer: { marginTop: space.lg, gap: space.sm, alignItems: 'center' },
  hint: { marginBottom: space.md, lineHeight: 20 },
  strong: { fontWeight: '600', color: m3.onSurface },
  mono: { fontFamily: 'monospace', color: m3.onSurface },
  center: { textAlign: 'center', color: m3.onSurfaceVariant },
  codeBox: {
    alignSelf: 'center',
    paddingHorizontal: space.xl,
    paddingVertical: space.lg,
    borderRadius: radius.lg,
    backgroundColor: m3.surfaceContainerHighest,
    marginVertical: space.md,
  },
  code: { fontSize: 40, letterSpacing: 6, fontWeight: '500', color: m3.onSurface, fontFamily: 'monospace' },
  waiting: { flexDirection: 'row', alignItems: 'center', justifyContent: 'center', gap: space.sm, marginVertical: space.md },
  done: { alignItems: 'center', gap: space.sm, paddingVertical: space.lg },
  sheetActions: { flexDirection: 'row', justifyContent: 'flex-end', gap: space.sm, marginTop: space.sm },
});
