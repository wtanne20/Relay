import { router } from 'expo-router';
import { useEffect, useState } from 'react';
import { ActivityIndicator, Pressable, ScrollView, StyleSheet, Text, View } from 'react-native';

import { describeError } from '../lib/api';
import { useConnection } from '../lib/connection';
import { useWide } from '../lib/layout';
import type { Device } from '../lib/types';
import { useBrainInfo, useHere } from './Location';
import { Sheet } from './Sheet';
import { m3, radius, space, type } from './theme';
import { Banner, Button, Field, Icon, IconButton, type IconName } from './ui';

/** Where a new project named `name` will live inside `root`. */
export function projectFolderPreview(root: string, name: string): string {
  const sep = root.includes('\\') ? '\\' : '/';
  const clean = name
    .replace(/[<>:"/\\|?*]/g, '-')
    .trim()
    .replace(/[. ]+$/, '');
  return `${root.replace(/[\\/]+$/, '')}${sep}${clean || '…'}`;
}

/**
 * New project. On the brain: just a name, and Relay creates a folder with
 * that name inside the projects folder set in Settings. On a linked
 * computer: pick one of its folders.
 */
export function AddProjectSheet({
  visible,
  onClose,
  onCreate,
}: {
  visible: boolean;
  onClose: () => void;
  onCreate: (name: string, path: string, deviceId?: string) => Promise<void>;
}) {
  const { client } = useConnection();
  const brain = useBrainInfo();
  const here = useHere();
  const [root, setRoot] = useState<string | null>(null);
  const [name, setName] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [devices, setDevices] = useState<Device[]>([]);
  /** '' = the brain; otherwise a device id. */
  const [where, setWhere] = useState('');
  const [folder, setFolder] = useState<{ path: string; root: string; folders: string[] } | null>(null);
  const [browsing, setBrowsing] = useState(false);

  useEffect(() => {
    if (!visible || !client) return;
    setError(null);
    client
      .settings()
      .then((r) => setRoot(r.settings.projects_folder ?? ''))
      .catch((e) => setError(describeError(e)));
    client
      .devices()
      .then((ds) => {
        setDevices(ds);
        // On a linked computer, its own folders are the likely choice.
        if (here.deviceId && ds.some((d) => d.id === here.deviceId)) setWhere(here.deviceId);
      })
      .catch(() => setDevices([]));
  }, [visible, client, here.deviceId]);

  const browse = async (deviceId: string, path: string) => {
    if (!client) return;
    setBrowsing(true);
    setError(null);
    try {
      setFolder(await client.browseDevice(deviceId, path));
    } catch (e) {
      setFolder(null);
      setError(describeError(e));
    } finally {
      setBrowsing(false);
    }
  };

  useEffect(() => {
    if (visible && where) browse(where, '');
    else setFolder(null);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [visible, where]);

  const submit = async () => {
    if (where) {
      if (!folder) return;
      setBusy(true);
      setError(null);
      try {
        await onCreate(name.trim(), folder.path, where);
        setName('');
      } catch (e) {
        setError(describeError(e));
      } finally {
        setBusy(false);
      }
      return;
    }
    if (!name.trim() || !root) return;
    setBusy(true);
    setError(null);
    try {
      await onCreate(name.trim(), '');
      setName('');
    } catch (e) {
      setError(describeError(e));
    } finally {
      setBusy(false);
    }
  };

  const wide = useWide();
  const openSettings = () => {
    onClose();
    // Desktop swaps the main pane; on the phone it's a normal screen you go back from.
    if (wide) router.replace('/settings');
    else router.push('/settings');
  };

  const sep = folder?.path.includes('\\') ? '\\' : '/';
  const join = (a: string, b: string) => `${a.replace(/[\\/]+$/, '')}${sep}${b}`;
  const parent = (p: string) => p.replace(/[\\/]+$/, '').replace(/[\\/][^\\/]*$/, '') || sep;
  const atRoot = !folder || folder.path.replace(/[\\/]+$/, '') === folder.root.replace(/[\\/]+$/, '');
  const brainName = brain?.hostname || 'the brain';
  const device = devices.find((d) => d.id === where);

  return (
    <Sheet visible={visible} onClose={onClose} title="New project">
      {devices.length > 0 ? (
        <>
          <Text style={[type.labelLarge, styles.label]}>Where the files are</Text>
          <View style={styles.places}>
            <Place
              icon="brain"
              label={here.isBrain ? 'This computer (brain)' : `${brainName} (brain)`}
              selected={where === ''}
              online
              onPress={() => setWhere('')}
            />
            {devices.map((d) => (
              <Place
                key={d.id}
                icon="laptop"
                label={d.id === here.deviceId ? `This computer (${d.name})` : d.name}
                selected={where === d.id}
                online={!!d.connected}
                onPress={() => setWhere(d.id)}
              />
            ))}
          </View>
        </>
      ) : null}
      {where ? (
        <>
          {device && !device.connected ? (
            <Banner message={`${device.name} is offline. Open Relay on it (and check Tailscale), then try again.`} />
          ) : null}
          {folder ? (
            <View style={styles.browser}>
              <View style={styles.crumbRow}>
                <IconButton
                  icon="arrow-up"
                  label="Up one folder"
                  size={18}
                  disabled={atRoot}
                  onPress={() => browse(where, parent(folder.path))}
                />
                <Text style={[type.bodyMedium, styles.grow]} numberOfLines={1} ellipsizeMode="head">
                  {folder.path}
                </Text>
                {browsing ? <ActivityIndicator size="small" color={m3.primary} /> : null}
              </View>
              <ScrollView style={styles.folderList}>
                {folder.folders.length === 0 ? (
                  <Text style={[type.bodySmall, styles.text]}>No folders inside.</Text>
                ) : null}
                {folder.folders.map((f) => (
                  <Pressable
                    key={f}
                    onPress={() => browse(where, join(folder.path, f))}
                    style={({ pressed }) => [styles.folderRow, pressed && styles.pressed]}
                    accessibilityRole="button"
                    accessibilityLabel={`Open ${f}`}
                  >
                    <Icon name="folder-outline" size={18} />
                    <Text style={[type.bodyMedium, styles.grow]} numberOfLines={1}>
                      {f}
                    </Text>
                    <Icon name="chevron-right" size={18} color={m3.outline} />
                  </Pressable>
                ))}
              </ScrollView>
            </View>
          ) : browsing ? (
            <ActivityIndicator color={m3.primary} style={styles.loading} />
          ) : null}
          <Field
            label="Project name (optional)"
            icon="tag-outline"
            placeholder={folder ? folder.path.split(/[\\/]/).filter(Boolean).pop() ?? '' : ''}
            value={name}
            onChangeText={setName}
            autoCapitalize="sentences"
            onSubmitEditing={submit}
          />
          {error ? <Banner message={error} /> : null}
          <View style={styles.actions}>
            <Button title="Cancel" variant="text" onPress={onClose} />
            <Button title="Use this folder" icon="check" onPress={submit} loading={busy} disabled={!folder} />
          </View>
        </>
      ) : root === null && !error ? (
        <ActivityIndicator color={m3.primary} style={styles.loading} />
      ) : root === '' ? (
        <>
          <Text style={[type.bodyMedium, styles.text]}>
            First choose the folder your projects live in. Each new project gets its own folder inside it.
          </Text>
          <View style={styles.actions}>
            <Button title="Cancel" variant="text" onPress={onClose} />
            <Button title="Open Settings" icon="cog-outline" onPress={openSettings} />
          </View>
        </>
      ) : (
        <>
          <Field
            label="Project name"
            icon="tag-outline"
            placeholder="Mileage tracker"
            value={name}
            onChangeText={setName}
            autoFocus
            autoCapitalize="sentences"
            onSubmitEditing={submit}
          />
          {root ? (
            <Text style={[type.bodySmall, styles.preview]} numberOfLines={2}>
              Creates {projectFolderPreview(root, name)} on {here.isBrain ? 'this computer' : brainName}
            </Text>
          ) : null}
          {error ? <Banner message={error} /> : null}
          <View style={styles.actions}>
            <Button title="Cancel" variant="text" onPress={onClose} />
            <Button title="Create" icon="check" onPress={submit} loading={busy} disabled={!name.trim()} />
          </View>
        </>
      )}
      {!where && root === null && error ? <Banner message={error} /> : null}
    </Sheet>
  );
}

function Place({
  icon,
  label,
  selected,
  online,
  onPress,
}: {
  icon: IconName;
  label: string;
  selected: boolean;
  online: boolean;
  onPress: () => void;
}) {
  return (
    <Pressable
      onPress={onPress}
      style={({ pressed }) => [styles.place, selected && styles.placeOn, pressed && styles.pressed]}
      accessibilityRole="radio"
      accessibilityState={{ selected }}
      accessibilityLabel={`${label}${online ? '' : ', offline'}`}
    >
      <Icon name={icon} size={18} color={selected ? m3.onSecondaryContainer : m3.onSurfaceVariant} />
      <Text style={[type.labelLarge, { color: selected ? m3.onSecondaryContainer : m3.onSurface }]} numberOfLines={1}>
        {label}
      </Text>
      {icon !== 'brain' ? (
        <View style={[styles.dot, { backgroundColor: online ? m3.success : m3.outline }]} />
      ) : null}
    </Pressable>
  );
}

const styles = StyleSheet.create({
  loading: { marginVertical: space.xl },
  text: { color: m3.onSurfaceVariant, marginBottom: space.md },
  preview: { color: m3.onSurfaceVariant, marginTop: -space.xs, marginBottom: space.sm, marginLeft: space.lg },
  actions: { flexDirection: 'row', justifyContent: 'flex-end', gap: space.sm, marginTop: space.sm },
  label: { color: m3.onSurfaceVariant, marginBottom: space.sm },
  places: { flexDirection: 'row', flexWrap: 'wrap', gap: space.sm, marginBottom: space.md },
  place: {
    flexDirection: 'row',
    alignItems: 'center',
    gap: 6,
    paddingHorizontal: space.md,
    paddingVertical: space.sm,
    borderRadius: radius.pill,
    borderWidth: 1,
    borderColor: m3.outlineVariant,
    maxWidth: '100%',
  },
  placeOn: { backgroundColor: m3.secondaryContainer, borderColor: m3.secondaryContainer },
  pressed: { opacity: 0.7 },
  dot: { width: 8, height: 8, borderRadius: 4 },
  browser: {
    borderWidth: 1,
    borderColor: m3.outlineVariant,
    borderRadius: radius.md,
    marginBottom: space.md,
    overflow: 'hidden',
  },
  crumbRow: {
    flexDirection: 'row',
    alignItems: 'center',
    gap: space.xs,
    paddingRight: space.md,
    backgroundColor: m3.surfaceContainerHigh,
  },
  folderList: { maxHeight: 240 },
  folderRow: { flexDirection: 'row', alignItems: 'center', gap: space.md, paddingHorizontal: space.md, paddingVertical: 10 },
  grow: { flex: 1, minWidth: 0 },
});
