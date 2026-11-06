import { router } from 'expo-router';
import { useCallback, useEffect, useState } from 'react';
import { ActivityIndicator, FlatList, Pressable, RefreshControl, StyleSheet, Text, View } from 'react-native';

import { Group, SectionTitle } from '../components/SettingsControls';
import { Sheet } from '../components/Sheet';
import { m3, radius, space, type } from '../components/theme';
import { alert } from '../components/Dialogs';
import { Banner, Button, Field, IconButton } from '../components/ui';
import { describeError } from '../lib/api';
import { useClient } from '../lib/connection';
import { usePageStyle } from '../lib/layout';
import type { Device } from '../lib/types';

export default function DevicesScreen() {
  const page = usePageStyle();
  const client = useClient();
  const [devices, setDevices] = useState<Device[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [refreshing, setRefreshing] = useState(false);
  const [adding, setAdding] = useState(false);
  const [pinging, setPinging] = useState<string | null>(null);

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
  }, [load]);

  if (!client) return null;

  const ping = async (d: Device) => {
    setPinging(d.id);
    try {
      const r = await client.pingDevice(d.id);
      if (!r.connected) alert('Not reachable', r.error ?? 'The device did not answer.', [{ text: 'OK' }]);
      await load();
    } catch (e) {
      alert('Not reachable', describeError(e), [{ text: 'OK' }]);
    } finally {
      setPinging(null);
    }
  };

  const remove = (d: Device) => {
    alert(
      `Remove ${d.name}?`,
      'It is forgotten on this PC. The device itself is not changed, and projects bound to it must be unbound or re-paired.',
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
            {error ? <Banner message={error} actionLabel="Retry" onAction={load} /> : null}
            <Text style={[type.bodyMedium, styles.intro]}>
              A device is a machine running the <Text style={type.bodyMedium}>relay host</Text> role — for example a
              phone with Proot Linux. The agent (on your PC) can read, write and run commands in the device's folder,
              using the PC's model. Pair one, then bind a project to it.
            </Text>
          </>
        }
        ListEmptyComponent={
          devices ? (
            <View style={styles.empty}>
              <Text style={type.bodyMedium}>No devices paired yet.</Text>
            </View>
          ) : undefined
        }
        renderItem={({ item }) => (
          <Group>
            <View style={styles.deviceRow}>
              <View style={styles.deviceIcon}>
                <Text style={type.titleMedium}>{item.os ? item.os.slice(0, 3) : 'dev'}</Text>
              </View>
              <View style={styles.grow}>
                <View style={styles.nameRow}>
                  <Text style={[type.bodyLarge, styles.name]} numberOfLines={1}>
                    {item.name}
                  </Text>
                  <View
                    style={[styles.dot, { backgroundColor: item.connected ? m3.primary : m3.outline }]}
                    accessibilityLabel={item.connected ? 'connected' : 'offline'}
                  />
                </View>
                <Text style={type.bodySmall} numberOfLines={1} ellipsizeMode="middle">
                  {item.root || item.url}
                </Text>
              </View>
              <IconButton
                icon={pinging === item.id ? 'timer-sand' : 'refresh'}
                label={`Check ${item.name}`}
                onPress={() => ping(item)}
              />
              <IconButton icon="trash-can-outline" label={`Remove ${item.name}`} onPress={() => remove(item)} />
            </View>
          </Group>
        )}
      />
      <Button title="Pair a device" icon="plus" onPress={() => setAdding(true)} style={styles.add} />
      <AddDeviceSheet
        visible={adding}
        onClose={() => setAdding(false)}
        onSaved={async (d) => {
          setAdding(false);
          setDevices((list) => [d, ...(list ?? [])]);
        }}
      />
    </View>
  );
}

function AddDeviceSheet({
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
    <Sheet visible={visible} onClose={onClose} title="Pair a device">
      <Text style={[type.bodySmall, styles.hint]}>
        On the device, start the <Text style={type.bodyMedium}>relay host</Text> role (for example{' '}
        <Text style={type.bodyMedium}>relay host --root /home/you/app</Text>). It prints the address and token; paste
        them here. The PC checks the link before saving.
      </Text>
      <Field label="Name" icon="cellphone" placeholder="phone" value={name} onChangeText={setName} />
      <Field
        label="Address"
        icon="lan-connect"
        placeholder="http://100.64.0.5:8766"
        hint="The device's Tailscale address and the host port (default 8766)."
        value={url}
        onChangeText={setUrl}
      />
      <Field
        label="Token"
        icon="key-outline"
        placeholder="the token relay host printed"
        value={token}
        onChangeText={setToken}
      />
      {error ? <Banner message={error} /> : null}
      <View style={styles.sheetActions}>
        <Button title="Cancel" variant="text" onPress={onClose} />
        <Button title="Pair" icon="check" onPress={save} loading={busy} disabled={!url.trim() || !token.trim()} />
      </View>
    </Sheet>
  );
}

const styles = StyleSheet.create({
  flex: { flex: 1, backgroundColor: m3.surface },
  content: { padding: space.lg, paddingBottom: 96 },
  intro: { marginBottom: space.md, marginHorizontal: space.xs, lineHeight: 22 },
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
  dot: { width: 8, height: 8, borderRadius: 4 },
  add: { position: 'absolute', bottom: space.xl, alignSelf: 'center', width: '80%' },
  hint: { marginBottom: space.md, lineHeight: 20 },
  sheetActions: { flexDirection: 'row', justifyContent: 'flex-end', gap: space.sm, marginTop: space.sm },
});
