import { useFocusEffect } from 'expo-router';
import { useCallback, useEffect, useRef, useState } from 'react';
import { ActivityIndicator, Pressable, ScrollView, StyleSheet, Text, View } from 'react-native';

import { alert } from '../components/Dialogs';
import { useBrainInfo, useHere } from '../components/Location';
import { Sheet } from '../components/Sheet';
import { m3, radius, space, type } from '../components/theme';
import { Banner, Button, EmptyState, Field, Icon, IconButton, type IconName } from '../components/ui';
import { describeError } from '../lib/api';
import { useClient } from '../lib/connection';
import { basename, relativeTime } from '../lib/format';
import { usePageStyle } from '../lib/layout';
import type { Device, DocumentFolder, DocumentSearch, FolderListing } from '../lib/types';

/**
 * Your documents: folders on any of your computers that the agent can
 * search and cite. Each computer indexes its own folders; only the passages
 * a search finds are sent to the brain.
 */
export default function DocumentsScreen() {
  const page = usePageStyle(820);
  const client = useClient();
  const here = useHere();
  const [folders, setFolders] = useState<DocumentFolder[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [adding, setAdding] = useState(false);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);

  const load = useCallback(async () => {
    if (!client) return;
    if (timer.current) clearTimeout(timer.current);
    try {
      const list = await client.documentFolders();
      setFolders(list);
      setError(null);
      // Follow indexing progress while a folder is being read.
      if (list.some((f) => f.status.indexing)) timer.current = setTimeout(load, 1500);
    } catch (e) {
      setError(describeError(e));
    }
  }, [client]);

  useFocusEffect(
    useCallback(() => {
      load();
      return () => {
        if (timer.current) clearTimeout(timer.current);
      };
    }, [load]),
  );

  if (!client) return null;

  const reindex = async (f: DocumentFolder) => {
    setBusy(f.id);
    try {
      await client.reindexDocumentFolder(f.id);
      await load();
    } catch (e) {
      alert('Could not reindex', describeError(e));
    } finally {
      setBusy(null);
    }
  };

  const remove = (f: DocumentFolder) =>
    alert(
      'Stop searching this folder?',
      `The agent won't search ${f.path} any more. Your files stay where they are; only Relay's index of them is deleted.`,
      [
        { text: 'Cancel', style: 'cancel' },
        {
          text: 'Remove',
          style: 'destructive',
          onPress: async () => {
            setBusy(f.id);
            try {
              await client.removeDocumentFolder(f.id);
              await load();
            } catch (e) {
              alert('Could not remove it', describeError(e));
            } finally {
              setBusy(null);
            }
          },
        },
      ],
    );

  const computerLabel = (f: DocumentFolder) => {
    if (!f.device_id) return here.isBrain ? `This computer (${f.computer})` : `${f.computer} (brain)`;
    return f.device_id === here.deviceId ? `This computer (${f.computer})` : f.computer;
  };

  return (
    <ScrollView contentContainerStyle={[styles.content, page]} keyboardShouldPersistTaps="handled">
      <Text style={[type.bodyMedium, styles.muted]}>
        Pick folders of notes, PDFs and documents on any of your computers. Relay indexes them on the computer they're on,
        and the agent can search them in any chat and tell you which file each answer came from. Your files never leave
        that computer: only the passages a search finds are sent to the model.
      </Text>
      {error ? <Banner message={error} actionLabel="Retry" onAction={load} /> : null}
      {!folders && !error ? <ActivityIndicator color={m3.primary} style={{ margin: space.xl }} /> : null}

      {folders && folders.length === 0 ? (
        <EmptyState
          icon="file-search-outline"
          title="No folders yet"
          body="Add a folder, like your Documents or a notes folder, and ask the agent about what's in it."
        />
      ) : null}

      {folders && folders.length > 0 ? (
        <View style={styles.card}>
          {folders.map((f, i) => (
            <FolderRow
              key={f.id}
              folder={f}
              first={i === 0}
              computer={computerLabel(f)}
              busy={busy === f.id}
              onReindex={() => reindex(f)}
              onRemove={() => remove(f)}
            />
          ))}
        </View>
      ) : null}

      {folders ? (
        <View style={styles.addRow}>
          <Button title="Add folder" icon="plus" variant="tonal" onPress={() => setAdding(true)} />
        </View>
      ) : null}

      {folders && folders.length > 0 ? <TrySearch /> : null}

      <AddFolderSheet
        visible={adding}
        taken={folders ?? []}
        onClose={() => setAdding(false)}
        onAdded={() => {
          setAdding(false);
          load();
        }}
      />
    </ScrollView>
  );
}

function statusLine(f: DocumentFolder): { text: string; tone: 'normal' | 'error' | 'busy' } {
  const s = f.status;
  if (!f.online) return { text: `${f.computer} is offline. Its folders are searched when it's back.`, tone: 'error' };
  if (s.error) return { text: s.error, tone: 'error' };
  if (s.indexing) {
    return {
      text: s.total ? `Indexing · ${s.done.toLocaleString()} of ${s.total.toLocaleString()} files` : 'Looking for changes…',
      tone: 'busy',
    };
  }
  if (!s.indexed_at) return { text: 'Waiting to be indexed', tone: 'busy' };
  const files = `${s.files.toLocaleString()} ${s.files === 1 ? 'file' : 'files'}`;
  const skipped = s.skipped ? ` · ${s.skipped.toLocaleString()} left out (too many files)` : '';
  return { text: `${files} · checked ${relativeTime(s.indexed_at)}${skipped}`, tone: 'normal' };
}

function FolderRow({
  folder: f,
  first,
  computer,
  busy,
  onReindex,
  onRemove,
}: {
  folder: DocumentFolder;
  first: boolean;
  computer: string;
  busy: boolean;
  onReindex: () => void;
  onRemove: () => void;
}) {
  const line = statusLine(f);
  const pct = f.status.total ? Math.min(1, f.status.done / f.status.total) : 0;
  return (
    <View style={[styles.folder, !first && styles.divider]}>
      <Icon name={f.device_id ? 'laptop' : 'brain'} size={22} color={m3.onSurfaceVariant} />
      <View style={styles.grow}>
        <Text style={type.bodyLarge} numberOfLines={1}>
          {basename(f.path)}
        </Text>
        <Text style={type.bodySmall} numberOfLines={1} ellipsizeMode="middle" selectable>
          {computer} · {f.path}
        </Text>
        <Text
          style={[
            type.bodySmall,
            line.tone === 'error' && { color: m3.error },
            line.tone === 'busy' && { color: m3.primary },
          ]}
          numberOfLines={3}
        >
          {line.text}
        </Text>
        {f.status.indexing && f.status.total ? (
          <View style={styles.track}>
            <View style={[styles.fill, { width: `${Math.round(pct * 100)}%` }]} />
          </View>
        ) : null}
      </View>
      {busy ? (
        <ActivityIndicator color={m3.primary} />
      ) : (
        <>
          <IconButton
            icon="refresh"
            label={`Check ${f.path} for changes now`}
            onPress={onReindex}
            disabled={!f.online || f.status.indexing}
          />
          <IconButton icon="close" label={`Stop searching ${f.path}`} onPress={onRemove} />
        </>
      )}
    </View>
  );
}

/** The same search the agent runs, to check what it will find. */
function TrySearch() {
  const client = useClient();
  const [q, setQ] = useState('');
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<DocumentSearch | null>(null);
  const [error, setError] = useState<string | null>(null);
  const run = async () => {
    if (!client || !q.trim()) return;
    setBusy(true);
    setError(null);
    try {
      setResult(await client.searchDocuments(q.trim()));
    } catch (e) {
      setError(describeError(e));
    } finally {
      setBusy(false);
    }
  };
  return (
    <View style={styles.try}>
      <Text style={type.titleMedium}>Try a search</Text>
      <Text style={[type.bodySmall, styles.muted]}>See what the agent finds for a few words.</Text>
      <View style={styles.searchRow}>
        <View style={styles.grow}>
          <Field
            label="Search your documents"
            icon="magnify"
            placeholder="hotel booking"
            value={q}
            onChangeText={setQ}
            onSubmitEditing={run}
            returnKeyType="search"
          />
        </View>
        <Button title="Search" onPress={run} loading={busy} disabled={!q.trim()} />
      </View>
      {error ? <Banner message={error} /> : null}
      {result ? (
        <View style={styles.card}>
          {result.hits.length === 0 ? (
            <Text style={[type.bodyMedium, styles.pad]}>Nothing matches. Try words that would appear in the text.</Text>
          ) : null}
          {result.hits.map((h, i) => (
            <View key={`${h.path}:${h.start}:${i}`} style={[styles.hit, i > 0 && styles.divider]}>
              <View style={styles.hitHead}>
                <Icon name={h.kind === 'pdf' ? 'file-pdf-box' : 'file-document-outline'} size={18} color={m3.primary} />
                <Text style={[type.labelLarge, styles.grow]} numberOfLines={1} ellipsizeMode="middle">
                  {h.rel}
                </Text>
                <Text style={[type.labelMedium, styles.muted]}>
                  {h.kind === 'pdf'
                    ? h.start === h.end
                      ? `p. ${h.start}`
                      : `pp. ${h.start}-${h.end}`
                    : h.start === h.end
                      ? `line ${h.start}`
                      : `lines ${h.start}-${h.end}`}
                </Text>
              </View>
              <Text style={[type.bodySmall, styles.muted]}>{h.computer}</Text>
              <Text style={type.bodyMedium} numberOfLines={5}>
                {h.text}
              </Text>
            </View>
          ))}
          {result.indexing ? (
            <Text style={[type.bodySmall, styles.pad, { color: m3.primary }]}>
              Some folders are still being indexed, so there may be more.
            </Text>
          ) : null}
          {result.notes.map((n) => (
            <Text key={n} style={[type.bodySmall, styles.pad, { color: m3.warning }]}>
              {n}
            </Text>
          ))}
        </View>
      ) : null}
    </View>
  );
}

function AddFolderSheet({
  visible,
  taken,
  onClose,
  onAdded,
}: {
  visible: boolean;
  taken: DocumentFolder[];
  onClose: () => void;
  onAdded: () => void;
}) {
  const client = useClient();
  const brain = useBrainInfo();
  const here = useHere();
  const [devices, setDevices] = useState<Device[]>([]);
  /** '' = the brain; otherwise a device id. */
  const [where, setWhere] = useState('');
  const [folder, setFolder] = useState<FolderListing | null>(null);
  const [browsing, setBrowsing] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!visible || !client) return;
    setError(null);
    client
      .devices()
      .then((ds) => {
        setDevices(ds);
        // On a linked computer, its own folders are the likely choice.
        setWhere(here.deviceId && ds.some((d) => d.id === here.deviceId) ? here.deviceId : '');
      })
      .catch(() => setDevices([]));
  }, [visible, client, here.deviceId]);

  const browse = useCallback(
    async (path: string) => {
      if (!client) return;
      setBrowsing(true);
      setError(null);
      try {
        setFolder(await client.browseFolders(where, path));
      } catch (e) {
        setError(describeError(e));
      } finally {
        setBrowsing(false);
      }
    },
    [client, where],
  );

  useEffect(() => {
    if (visible) {
      setFolder(null);
      browse('');
    }
  }, [visible, browse]);

  const add = async () => {
    if (!client || !folder) return;
    setBusy(true);
    setError(null);
    try {
      await client.addDocumentFolder(folder.path, where);
      onAdded();
    } catch (e) {
      setError(describeError(e));
    } finally {
      setBusy(false);
    }
  };

  const sep = folder?.path.includes('\\') ? '\\' : '/';
  const join = (a: string, b: string) => `${a.replace(/[\\/]+$/, '')}${sep}${b}`;
  const parent = (p: string) => {
    const up = p.replace(/[\\/]+$/, '').replace(/[\\/][^\\/]*$/, '');
    // "C:" alone means the current folder on that drive; keep the slash.
    return /^[A-Za-z]:$/.test(up) ? `${up}\\` : up || sep;
  };
  const atRoot = !folder || folder.path.replace(/[\\/]+$/, '') === folder.root.replace(/[\\/]+$/, '');
  const device = devices.find((d) => d.id === where);
  const already = folder
    ? taken.find((t) => t.device_id === where && samePath(t.path, folder.path))
    : undefined;

  return (
    <Sheet visible={visible} onClose={onClose} title="Add a folder to search">
      {devices.length > 0 ? (
        <>
          <Text style={[type.labelLarge, styles.label]}>Computer</Text>
          <View style={styles.places}>
            <Place
              icon="brain"
              label={here.isBrain ? 'This computer (brain)' : `${brain?.hostname || 'Brain'} (brain)`}
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
              onPress={() => browse(parent(folder.path))}
            />
            <Text style={[type.bodyMedium, styles.grow]} numberOfLines={1} ellipsizeMode="head">
              {folder.path}
            </Text>
            {browsing ? <ActivityIndicator size="small" color={m3.primary} /> : null}
          </View>
          <ScrollView style={styles.folderList}>
            {folder.folders.length === 0 ? (
              <Text style={[type.bodySmall, styles.pad]}>No folders inside.</Text>
            ) : null}
            {folder.folders.map((name) => (
              <Pressable
                key={name}
                onPress={() => browse(join(folder.path, name))}
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
      ) : browsing ? (
        <ActivityIndicator color={m3.primary} style={{ marginVertical: space.xl }} />
      ) : null}
      <Text style={[type.bodySmall, styles.muted]}>
        Relay reads text, Markdown, PDF and Word files (and code). Hidden folders and tool folders like node_modules are
        skipped.
      </Text>
      {already ? <Text style={[type.bodySmall, { color: m3.warning }]}>This folder is already searchable.</Text> : null}
      {error ? <Banner message={error} /> : null}
      <View style={styles.sheetActions}>
        <Button title="Cancel" variant="text" onPress={onClose} />
        <Button
          title="Search this folder"
          icon="check"
          onPress={add}
          loading={busy}
          disabled={!folder || !!already || (device && !device.connected)}
        />
      </View>
    </Sheet>
  );
}

function samePath(a: string, b: string) {
  const n = (s: string) => s.replace(/\\/g, '/').replace(/\/+$/, '').toLowerCase();
  return n(a) === n(b);
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
  content: { padding: space.lg, gap: space.md },
  muted: { color: m3.onSurfaceVariant },
  grow: { flex: 1, minWidth: 0 },
  pad: { padding: space.lg },
  card: { backgroundColor: m3.surfaceContainerLow, borderRadius: radius.lg, overflow: 'hidden' },
  folder: {
    flexDirection: 'row',
    alignItems: 'center',
    gap: space.md,
    paddingLeft: space.lg,
    paddingRight: space.xs,
    paddingVertical: space.md,
  },
  divider: { borderTopWidth: 1, borderTopColor: m3.outlineVariant },
  track: { height: 4, borderRadius: 2, backgroundColor: m3.surfaceContainerHighest, marginTop: space.xs },
  fill: { height: 4, borderRadius: 2, backgroundColor: m3.primary },
  addRow: { flexDirection: 'row' },
  try: { gap: space.sm, marginTop: space.lg },
  searchRow: { flexDirection: 'row', alignItems: 'center', gap: space.sm },
  hit: { padding: space.lg, gap: 2 },
  hitHead: { flexDirection: 'row', alignItems: 'center', gap: space.sm },
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
  folderRow: { flexDirection: 'row', alignItems: 'center', gap: space.md, paddingHorizontal: space.md, paddingVertical: 10 },
  sheetActions: { flexDirection: 'row', justifyContent: 'flex-end', gap: space.sm, marginTop: space.md },
});
