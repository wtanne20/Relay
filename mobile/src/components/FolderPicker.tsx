import { useCallback, useEffect, useRef, useState } from 'react';
import { ActivityIndicator, Pressable, ScrollView, StyleSheet, Text, View } from 'react-native';

import { describeError } from '../lib/api';
import { useClient } from '../lib/connection';
import type { Device, FolderListing } from '../lib/types';
import { useBrainInfo, useHere } from './Location';
import { m3, radius, space, type } from './theme';
import { Banner, Icon, IconButton, type IconName } from './ui';

/**
 * Picking a folder on the brain or a linked computer: the row of computers,
 * and a browser that walks down (and back up) that computer's folders. Used
 * by "New project" and by "Your documents", so the two always behave alike.
 */

/** Two paths name the same folder (ignoring slashes, a trailing slash and case). */
export function samePath(a: string, b: string): boolean {
  const n = (s: string) => s.replace(/\\/g, '/').replace(/\/+$/, '').toLowerCase();
  return n(a) === n(b);
}

function separator(path: string): string {
  return path.includes('\\') ? '\\' : '/';
}

/** `name` inside the folder `dir`, with that computer's slash. */
export function joinPath(dir: string, name: string): string {
  return `${dir.replace(/[\\/]+$/, '')}${separator(dir)}${name}`;
}

/** The folder above `path`. Keeps the slash on a bare drive ("C:\"), since "C:" means the current folder there. */
export function parentPath(path: string): string {
  const up = path.replace(/[\\/]+$/, '').replace(/[\\/][^\\/]*$/, '');
  return /^[A-Za-z]:$/.test(up) ? `${up}\\` : up || separator(path);
}

/**
 * The computers a folder can be on. Loads the linked computers each time
 * `visible` turns on, and starts on this computer when it is a linked one
 * (its own folders are the likely choice), otherwise on the brain.
 */
export function useComputers(visible: boolean) {
  const client = useClient();
  const here = useHere();
  const [devices, setDevices] = useState<Device[]>([]);
  /** '' = the brain; otherwise a device id. */
  const [where, setWhere] = useState('');

  useEffect(() => {
    if (!visible || !client) return;
    client
      .devices()
      .then((ds) => {
        setDevices(ds);
        setWhere(here.deviceId && ds.some((d) => d.id === here.deviceId) ? here.deviceId : '');
      })
      .catch(() => setDevices([]));
  }, [visible, client, here.deviceId]);

  return { devices, where, setWhere, device: devices.find((d) => d.id === where) };
}

/**
 * One folder listing at a time. `load` fetches a folder ('' = where the
 * computer's shared folder starts); errors go to `onError`.
 */
export function useFolderBrowser(
  load: ((path: string) => Promise<FolderListing>) | null,
  onError: (message: string | null) => void,
) {
  const [folder, setFolder] = useState<FolderListing | null>(null);
  const [browsing, setBrowsing] = useState(false);
  /** Only the newest request wins, so a slow answer can't replace a newer folder. */
  const seq = useRef(0);

  const browse = useCallback(
    async (path: string) => {
      if (!load) return;
      const mine = ++seq.current;
      setBrowsing(true);
      onError(null);
      try {
        const next = await load(path);
        if (mine === seq.current) setFolder(next);
      } catch (e) {
        if (mine === seq.current) onError(describeError(e));
      } finally {
        if (mine === seq.current) setBrowsing(false);
      }
    },
    // onError is a state setter in practice; leaving it out keeps `browse` stable.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [load],
  );

  const clear = useCallback(() => {
    seq.current++;
    setFolder(null);
    setBrowsing(false);
  }, []);

  return { folder, browsing, browse, clear };
}

/** The brain and each linked computer, as a row of choices. */
export function ComputerPicker({
  label,
  devices,
  value,
  onChange,
}: {
  label: string;
  devices: Device[];
  /** '' = the brain. */
  value: string;
  onChange: (deviceId: string) => void;
}) {
  const brain = useBrainInfo();
  const here = useHere();
  if (devices.length === 0) return null;
  return (
    <>
      <Text style={[type.labelLarge, styles.label]}>{label}</Text>
      <View style={styles.places}>
        <Place
          icon="brain"
          label={here.isBrain ? 'This computer (brain)' : `${brain?.hostname || 'Brain'} (brain)`}
          selected={value === ''}
          online
          onPress={() => onChange('')}
        />
        {devices.map((d) => (
          <Place
            key={d.id}
            icon="laptop"
            label={d.id === here.deviceId ? `This computer (${d.name})` : d.name}
            selected={value === d.id}
            online={!!d.connected}
            onPress={() => onChange(d.id)}
          />
        ))}
      </View>
    </>
  );
}

/** Says so when the chosen linked computer is offline. */
export function OfflineBanner({ device }: { device: Device | undefined }) {
  if (!device || device.connected) return null;
  return <Banner message={`${device.name} is offline. Open Relay on it (and check Tailscale), then try again.`} />;
}

/** The current folder with an "up" button, and the folders inside it to open. */
export function FolderBrowser({
  folder,
  browsing,
  onBrowse,
}: {
  folder: FolderListing | null;
  browsing: boolean;
  onBrowse: (path: string) => void;
}) {
  if (!folder) return browsing ? <ActivityIndicator color={m3.primary} style={styles.loading} /> : null;
  const atRoot = folder.path.replace(/[\\/]+$/, '') === folder.root.replace(/[\\/]+$/, '');
  return (
    <View style={styles.browser}>
      <View style={styles.crumbRow}>
        <IconButton
          icon="arrow-up"
          label="Up one folder"
          size={18}
          disabled={atRoot}
          onPress={() => onBrowse(parentPath(folder.path))}
        />
        <Text style={[type.bodyMedium, styles.grow]} numberOfLines={1} ellipsizeMode="head">
          {folder.path}
        </Text>
        {browsing ? <ActivityIndicator size="small" color={m3.primary} /> : null}
      </View>
      <ScrollView style={styles.folderList}>
        {folder.folders.length === 0 ? <Text style={[type.bodySmall, styles.empty]}>No folders inside.</Text> : null}
        {folder.folders.map((name) => (
          <Pressable
            key={name}
            onPress={() => onBrowse(joinPath(folder.path, name))}
            style={({ pressed }) => [styles.folderRow, pressed && styles.pressed]}
            accessibilityRole="button"
            accessibilityLabel={`Open ${name}`}
          >
            <Icon name="folder-outline" size={18} />
            <Text style={[type.bodyMedium, styles.grow]} numberOfLines={1}>
              {name}
            </Text>
            <Icon name="chevron-right" size={18} color={m3.outline} />
          </Pressable>
        ))}
      </ScrollView>
    </View>
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
      {icon !== 'brain' ? <View style={[styles.dot, { backgroundColor: online ? m3.success : m3.outline }]} /> : null}
    </Pressable>
  );
}

const styles = StyleSheet.create({
  loading: { marginVertical: space.xl },
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
  folderList: { maxHeight: 260 },
  folderRow: {
    flexDirection: 'row',
    alignItems: 'center',
    gap: space.md,
    paddingHorizontal: space.md,
    paddingVertical: 10,
  },
  empty: { color: m3.onSurfaceVariant, padding: space.lg },
  grow: { flex: 1, minWidth: 0 },
});
