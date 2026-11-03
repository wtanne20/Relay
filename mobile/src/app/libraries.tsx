import { useFocusEffect } from 'expo-router';
import { useCallback, useEffect, useRef, useState } from 'react';
import { ActivityIndicator, ScrollView, StyleSheet, Text, View } from 'react-native';

import { alert } from '../components/Dialogs';
import { ToggleRow } from '../components/SettingsControls';
import { Sheet } from '../components/Sheet';
import { m3, radius, space, type } from '../components/theme';
import { Banner, Button, Field, Icon, IconButton, type IconName } from '../components/ui';
import { describeError } from '../lib/api';
import { useClient } from '../lib/connection';
import { formatBytes } from '../lib/format';
import { usePageStyle } from '../lib/layout';
import type { Disk, HubDownload, ModelLibrary } from '../lib/types';

/** Room a move needs on top of the model itself (matches the server). */
const SPARE = 512 * 1024 * 1024;

const DISK_ICON: Record<Disk['kind'], IconName> = {
  local: 'harddisk',
  network: 'nas',
  removable: 'usb-flash-drive-outline',
  unknown: 'harddisk',
};

function diskNote(d: Disk): string | null {
  if (d.kind === 'network') return 'Network drive. Models load slowly from here.';
  if (d.kind === 'removable') return 'Removable drive. Models here are missing while it is unplugged.';
  return null;
}

function sameDisk(a: ModelLibrary, b: ModelLibrary) {
  return (a.disk?.root ?? a.path) === (b.disk?.root ?? b.path);
}

/** Model libraries: folders on any disk that hold models, like Steam libraries. */
export default function LibrariesScreen() {
  const page = usePageStyle(820);
  const client = useClient();
  const [libs, setLibs] = useState<ModelLibrary[] | null>(null);
  const [current, setCurrent] = useState('');
  const [moves, setMoves] = useState<HubDownload[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [adding, setAdding] = useState(false);
  const [moving, setMoving] = useState<{ path: string; name: string; size: number; from: string } | null>(null);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const seen = useRef(new Set<string>());
  const primed = useRef(false);
  const old = useRef(new Set<string>());

  const load = useCallback(async () => {
    if (!client) return;
    try {
      const r = await client.libraries();
      setLibs(r.libraries);
      setCurrent(r.current_model);
      setError(null);
    } catch (e) {
      setError(describeError(e));
    }
  }, [client]);

  const pollMoves = useCallback(async () => {
    if (!client) return;
    if (timer.current) clearTimeout(timer.current);
    try {
      const d = (await client.hubDownloads()).downloads.filter((x) => x.repo === 'move');
      // A move finished since the last look: its files have moved (the server
      // removes the originals a moment later), so list the libraries again.
      const finished = d.filter((x) => x.state === 'done' && !seen.current.has(x.id));
      finished.forEach((x) => seen.current.add(x.id));
      // Moves that ended before this page opened are old news.
      if (!primed.current) d.filter((x) => x.state !== 'downloading').forEach((x) => old.current.add(x.id));
      if (finished.length && primed.current) {
        load();
        setTimeout(load, 3000);
        setTimeout(load, 8000);
      }
      primed.current = true;
      setMoves(d);
      if (d.some((x) => x.state === 'downloading')) timer.current = setTimeout(pollMoves, 1000);
    } catch {
      timer.current = setTimeout(pollMoves, 3000);
    }
  }, [client, load]);

  useFocusEffect(
    useCallback(() => {
      load();
      pollMoves();
      return () => {
        if (timer.current) clearTimeout(timer.current);
      };
    }, [load, pollMoves]),
  );

  if (!client) return null;

  const run = async (key: string, act: () => Promise<{ libraries: ModelLibrary[] }>) => {
    setBusy(key);
    try {
      setLibs((await act()).libraries);
    } catch (e) {
      alert('Something went wrong', describeError(e));
    } finally {
      setBusy(null);
    }
  };

  const remove = (lib: ModelLibrary) =>
    alert(
      'Remove this library?',
      `Relay stops using ${lib.path}. The folder and the ${lib.models.length === 1 ? 'model' : 'models'} in it stay on the disk, and you can add it again later.`,
      [
        { text: 'Cancel', style: 'cancel' },
        { text: 'Remove', style: 'destructive', onPress: () => run(lib.path, () => client.removeLibrary(lib.path)) },
      ],
    );

  // Group libraries by the disk they're on.
  const disks: { disk: Disk | null; libs: ModelLibrary[] }[] = [];
  for (const lib of libs ?? []) {
    const g = disks.find((d) => sameDisk(d.libs[0], lib));
    if (g) g.libs.push(lib);
    else disks.push({ disk: lib.disk, libs: [lib] });
  }
  const activeMoves = moves.filter((m) => m.state !== 'cancelled' && !old.current.has(m.id));
  const movingTo = (path: string) => moves.find((m) => m.state === 'downloading' && m.file === basename(path));

  return (
    <ScrollView contentContainerStyle={[styles.content, page]}>
      <Text style={[type.bodyMedium, styles.muted]}>
        Libraries are folders Relay keeps models in. Add one on any drive, pick which one new downloads go to, and move
        models between them to free up space.
      </Text>
      {error ? <Banner message={error} actionLabel="Retry" onAction={load} /> : null}
      {!libs && !error ? <ActivityIndicator color={m3.primary} style={{ margin: space.xl }} /> : null}

      {activeMoves.length ? (
        <View style={styles.card}>
          {activeMoves.map((m) => (
            <MoveRow key={m.id} m={m} onCancel={() => client.hubCancel(m.id).then(pollMoves)} />
          ))}
        </View>
      ) : null}

      {disks.map(({ disk, libs: onDisk }) => (
        <View key={disk?.root ?? onDisk[0].path} style={styles.card}>
          <DiskHeader disk={disk} relayBytes={onDisk.reduce((n, l) => n + l.size_bytes, 0)} />
          {onDisk.map((lib) => (
            <View key={lib.path} style={styles.library}>
              <View style={styles.libHead}>
                <Icon name="folder-outline" size={20} color={m3.onSurfaceVariant} />
                <View style={styles.grow}>
                  <Text style={type.bodyLarge} numberOfLines={2} selectable>
                    {lib.path}
                  </Text>
                  <View style={styles.badges}>
                    {lib.default ? <Badge label="New downloads go here" strong /> : null}
                    {lib.builtin ? <Badge label="Built in" /> : null}
                    {!lib.exists ? <Badge label="Folder not found" warn /> : null}
                    <Text style={type.bodySmall}>
                      {lib.models.length} {lib.models.length === 1 ? 'model' : 'models'} · {formatBytes(lib.size_bytes)}
                    </Text>
                  </View>
                </View>
                {busy === lib.path ? (
                  <ActivityIndicator color={m3.primary} />
                ) : (
                  <>
                    {!lib.default ? (
                      <Button
                        title="Make default"
                        variant="text"
                        onPress={() => run(lib.path, () => client.setDefaultLibrary(lib.path))}
                      />
                    ) : null}
                    {!lib.builtin ? (
                      <IconButton icon="close" label={`Remove library ${lib.path}`} onPress={() => remove(lib)} />
                    ) : null}
                  </>
                )}
              </View>
              {lib.models.map((model) => {
                const mv = movingTo(model.path);
                return (
                  <View key={model.path} style={styles.model}>
                    <Icon name="chip" size={18} color={m3.onSurfaceVariant} />
                    <View style={styles.grow}>
                      <Text style={type.bodyMedium} numberOfLines={1} ellipsizeMode="middle">
                        {model.name}
                      </Text>
                      <Text style={type.bodySmall}>
                        {formatBytes(model.size_bytes)}
                        {current && samePath(current, model.path) ? ' · In use' : ''}
                      </Text>
                    </View>
                    {mv ? (
                      <Text style={[type.labelMedium, styles.muted]}>Moving…</Text>
                    ) : (libs?.length ?? 0) > 1 ? (
                      <Button
                        title="Move"
                        variant="text"
                        icon="folder-move-outline"
                        onPress={() =>
                          setMoving({ path: model.path, name: model.name, size: model.size_bytes, from: lib.path })
                        }
                      />
                    ) : null}
                  </View>
                );
              })}
            </View>
          ))}
        </View>
      ))}

      {libs ? (
        <View style={styles.addRow}>
          <Button title="Add library" icon="plus" variant="tonal" onPress={() => setAdding(true)} />
        </View>
      ) : null}

      <AddLibrary
        visible={adding}
        onClose={() => setAdding(false)}
        onAdded={(l) => {
          setLibs(l);
          setAdding(false);
        }}
      />
      <MoveSheet
        model={moving}
        libs={libs ?? []}
        onClose={() => setMoving(null)}
        onStarted={() => {
          setMoving(null);
          pollMoves();
        }}
      />
    </ScrollView>
  );
}

function basename(p: string) {
  const parts = p.split(/[\\/]+/).filter(Boolean);
  return parts[parts.length - 1] ?? p;
}

function samePath(a: string, b: string) {
  const n = (s: string) => s.replace(/\//g, '\\').replace(/\\+$/, '').toLowerCase();
  return n(a) === n(b);
}

function Badge({ label, strong, warn }: { label: string; strong?: boolean; warn?: boolean }) {
  return (
    <View style={[styles.badge, strong && styles.badgeStrong, warn && styles.badgeWarn]}>
      <Text
        style={[
          type.labelMedium,
          { color: strong ? m3.onSecondaryContainer : warn ? m3.onErrorContainer : m3.onSurfaceVariant },
        ]}
      >
        {label}
      </Text>
    </View>
  );
}

/** The drive, with a bar: Relay's models, everything else, and free space. */
function DiskHeader({ disk, relayBytes }: { disk: Disk | null; relayBytes: number }) {
  if (!disk) {
    return (
      <View style={styles.diskHead}>
        <Icon name="harddisk" size={24} color={m3.onSurfaceVariant} />
        <Text style={[type.titleMedium, styles.grow]}>Drive not available</Text>
      </View>
    );
  }
  const total = Math.max(1, disk.total_bytes);
  const used = Math.max(0, disk.total_bytes - disk.free_bytes);
  const relay = Math.min(relayBytes, used);
  const pct = (n: number) => `${Math.max(0, Math.min(100, (n / total) * 100))}%` as const;
  const low = disk.free_bytes < total * 0.1;
  const note = diskNote(disk);
  return (
    <View style={styles.disk}>
      <View style={styles.diskHead}>
        <Icon name={DISK_ICON[disk.kind]} size={24} color={m3.onSurfaceVariant} />
        <Text style={[type.titleMedium, styles.grow]} numberOfLines={1}>
          {disk.root}
        </Text>
        <Text style={[type.labelLarge, { color: low ? m3.error : m3.onSurfaceVariant }]}>
          {formatBytes(disk.free_bytes)} free of {formatBytes(disk.total_bytes)}
        </Text>
      </View>
      <View style={styles.bar} accessibilityLabel={`${Math.round((used / total) * 100)}% used`}>
        <View style={[styles.barRelay, { width: pct(relay) }]} />
        <View style={[styles.barOther, { width: pct(used - relay) }, low && { backgroundColor: m3.error }]} />
      </View>
      <View style={styles.legend}>
        <View style={[styles.dot, { backgroundColor: m3.primary }]} />
        <Text style={type.bodySmall}>Relay models {formatBytes(relayBytes)}</Text>
        <View style={[styles.dot, { backgroundColor: low ? m3.error : m3.outline }]} />
        <Text style={type.bodySmall}>Other files {formatBytes(Math.max(0, used - relay))}</Text>
      </View>
      {note ? <Text style={[type.bodySmall, { color: m3.warning }]}>{note}</Text> : null}
    </View>
  );
}

function MoveRow({ m, onCancel }: { m: HubDownload; onCancel: () => void }) {
  const pct = m.total_bytes ? Math.min(1, m.done_bytes / m.total_bytes) : 0;
  const status =
    m.state === 'downloading'
      ? `Moving · ${formatBytes(m.done_bytes)} of ${formatBytes(m.total_bytes)} · ${Math.round(pct * 100)}%`
      : m.state === 'done'
        ? 'Moved. Chats using it now use the new place.'
        : `Move failed: ${m.error ?? 'unknown error'}. The original is untouched.`;
  return (
    <View style={styles.model}>
      <Icon
        name={
          m.state === 'done'
            ? 'check-circle-outline'
            : m.state === 'failed'
              ? 'alert-circle-outline'
              : 'folder-move-outline'
        }
        size={22}
        color={m.state === 'done' ? m3.success : m.state === 'failed' ? m3.error : m3.onSurfaceVariant}
      />
      <View style={styles.grow}>
        <Text style={type.bodyLarge} numberOfLines={1} ellipsizeMode="middle">
          {m.file}
        </Text>
        <Text style={[type.bodySmall, m.state === 'failed' && { color: m3.error }]} numberOfLines={2}>
          {status}
        </Text>
        {m.state === 'downloading' ? (
          <View style={styles.track}>
            <View style={[styles.fill, { width: `${Math.round(pct * 100)}%` }]} />
          </View>
        ) : null}
      </View>
      {m.state === 'downloading' ? (
        <IconButton icon="close" label={`Cancel moving ${m.file}`} onPress={onCancel} />
      ) : null}
    </View>
  );
}

function AddLibrary({
  visible,
  onClose,
  onAdded,
}: {
  visible: boolean;
  onClose: () => void;
  onAdded: (libs: ModelLibrary[]) => void;
}) {
  const client = useClient();
  const [path, setPath] = useState('');
  const [makeDefault, setMakeDefault] = useState(true);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    if (visible) {
      setPath('');
      setError(null);
      setMakeDefault(true);
    }
  }, [visible]);
  const save = async () => {
    if (!client || !path.trim()) return;
    setBusy(true);
    setError(null);
    try {
      onAdded((await client.addLibrary(path.trim(), makeDefault)).libraries);
    } catch (e) {
      setError(describeError(e));
    } finally {
      setBusy(false);
    }
  };
  return (
    <Sheet visible={visible} onClose={onClose} title="Add library">
      <Field
        label="Folder on your PC"
        icon="folder-plus-outline"
        placeholder="W:\Relay\models"
        value={path}
        onChangeText={setPath}
        hint="Any folder on any drive. It's created if it doesn't exist. Models already in it show up right away."
        autoFocus
        onSubmitEditing={save}
      />
      <ToggleRow label="Save new downloads here" value={makeDefault} onChange={setMakeDefault} />
      {error ? <Banner message={error} /> : null}
      <View style={styles.sheetActions}>
        <Button title="Cancel" variant="text" onPress={onClose} />
        <Button title="Add" icon="check" onPress={save} loading={busy} disabled={!path.trim()} />
      </View>
    </Sheet>
  );
}

function MoveSheet({
  model,
  libs,
  onClose,
  onStarted,
}: {
  model: { path: string; name: string; size: number; from: string } | null;
  libs: ModelLibrary[];
  onClose: () => void;
  onStarted: () => void;
}) {
  const client = useClient();
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => setError(null), [model]);
  const targets = libs.filter((l) => model && !samePath(l.path, model.from));
  const move = async (lib: ModelLibrary) => {
    if (!client || !model) return;
    setBusy(lib.path);
    setError(null);
    try {
      await client.moveModel(model.path, lib.path);
      onStarted();
    } catch (e) {
      setError(describeError(e));
    } finally {
      setBusy(null);
    }
  };
  return (
    <Sheet visible={model !== null} onClose={onClose} title="Move model">
      <Text style={[type.bodyMedium, styles.muted]}>
        {model ? `${model.name} (${formatBytes(model.size)})` : ''}. Relay copies it, checks the copy, then removes the
        original. Chats that use it keep working.
      </Text>
      <View style={styles.targets}>
        {targets.map((lib) => {
          const room = !lib.disk || lib.disk.free_bytes >= (model?.size ?? 0) + SPARE;
          const here =
            lib.disk && libs.find((l) => model && samePath(l.path, model.from))?.disk?.root === lib.disk.root;
          return (
            <View key={lib.path} style={styles.target}>
              <Icon name={lib.disk ? DISK_ICON[lib.disk.kind] : 'harddisk'} size={20} color={m3.onSurfaceVariant} />
              <View style={styles.grow}>
                <Text style={type.bodyLarge} numberOfLines={2}>
                  {lib.path}
                </Text>
                <Text style={[type.bodySmall, !room && { color: m3.error }]}>
                  {lib.disk ? `${formatBytes(lib.disk.free_bytes)} free` : 'Drive not available'}
                  {!room ? ' · not enough space' : here ? ' · same drive, frees no space' : ''}
                </Text>
              </View>
              <Button
                title="Move here"
                variant="tonal"
                onPress={() => move(lib)}
                loading={busy === lib.path}
                disabled={!room || busy !== null || !lib.disk}
              />
            </View>
          );
        })}
      </View>
      {error ? <Banner message={error} /> : null}
      <View style={styles.sheetActions}>
        <Button title="Cancel" variant="text" onPress={onClose} />
      </View>
    </Sheet>
  );
}

const styles = StyleSheet.create({
  content: { padding: space.lg, gap: space.md },
  muted: { color: m3.onSurfaceVariant },
  grow: { flex: 1, minWidth: 0 },
  card: { backgroundColor: m3.surfaceContainerLow, borderRadius: radius.lg, overflow: 'hidden' },
  disk: { padding: space.lg, gap: space.sm },
  diskHead: { flexDirection: 'row', alignItems: 'center', gap: space.md, flexWrap: 'wrap' },
  bar: {
    flexDirection: 'row',
    height: 10,
    borderRadius: 5,
    backgroundColor: m3.surfaceContainerHighest,
    overflow: 'hidden',
  },
  barRelay: { height: '100%', backgroundColor: m3.primary },
  barOther: { height: '100%', backgroundColor: m3.outline },
  legend: { flexDirection: 'row', alignItems: 'center', gap: space.xs, flexWrap: 'wrap' },
  dot: { width: 8, height: 8, borderRadius: 4, marginLeft: space.xs },
  library: { borderTopWidth: 1, borderTopColor: m3.outlineVariant, paddingVertical: space.sm },
  libHead: {
    flexDirection: 'row',
    alignItems: 'center',
    gap: space.md,
    paddingHorizontal: space.lg,
    paddingVertical: space.xs,
  },
  badges: { flexDirection: 'row', alignItems: 'center', gap: space.sm, flexWrap: 'wrap', marginTop: 2 },
  badge: {
    paddingHorizontal: space.sm,
    paddingVertical: 2,
    borderRadius: radius.sm,
    backgroundColor: m3.surfaceContainerHighest,
  },
  badgeStrong: { backgroundColor: m3.secondaryContainer },
  badgeWarn: { backgroundColor: m3.errorContainer },
  model: {
    flexDirection: 'row',
    alignItems: 'center',
    gap: space.md,
    minHeight: 48,
    paddingLeft: space.lg + 36,
    paddingRight: space.sm,
  },
  track: { height: 4, borderRadius: 2, backgroundColor: m3.surfaceContainerHighest, marginTop: space.xs },
  fill: { height: 4, borderRadius: 2, backgroundColor: m3.primary },
  addRow: { flexDirection: 'row' },
  targets: { gap: space.sm, marginVertical: space.md },
  target: { flexDirection: 'row', alignItems: 'center', gap: space.md },
  sheetActions: { flexDirection: 'row', justifyContent: 'flex-end', gap: space.sm, marginTop: space.md },
});
