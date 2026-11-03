import { router } from 'expo-router';
import { useCallback, useEffect, useRef, useState } from 'react';
import { ActivityIndicator, Pressable, ScrollView, StyleSheet, Text, View } from 'react-native';

import { Sheet } from '../components/Sheet';
import { m3, radius, space, type } from '../components/theme';
import { Banner, Button, Field, Icon, IconButton } from '../components/ui';
import { describeError } from '../lib/api';
import { useConnection } from '../lib/connection';
import { formatBytes } from '../lib/format';
import { usePageStyle } from '../lib/layout';
import type { HubDownload, HubFile, HubRepo, ModelLibrary } from '../lib/types';

const SUGGESTIONS = ['Qwen3 Coder', 'Devstral', 'GLM', 'gpt-oss', 'Gemma', 'Mistral'];

/** VRAM left for weights after the KV cache and display (rough). */
function fit(size: number, vramMb: number): { label: string; color: string } | null {
  if (!vramMb) return null;
  const vram = vramMb * 1024 * 1024;
  if (size <= vram - 6 * 1024 ** 3) return { label: 'Fits your GPU', color: m3.success };
  if (size <= vram - 2.5 * 1024 ** 3) return { label: 'Tight fit', color: m3.warning };
  return { label: 'Too big for your GPU', color: m3.error };
}

function compact(n: number): string {
  if (n >= 1e6) return `${(n / 1e6).toFixed(1)}M`;
  if (n >= 1e3) return `${Math.round(n / 1e3)}k`;
  return String(n);
}

/** Search Hugging Face for GGUF models and download them to the PC. */
export default function HubScreen() {
  const page = usePageStyle(820);
  const { client } = useConnection();
  const [query, setQuery] = useState('');
  const [results, setResults] = useState<HubRepo[] | null>(null);
  const [searching, setSearching] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [open, setOpen] = useState<string | null>(null);
  const [downloads, setDownloads] = useState<HubDownload[]>([]);
  const [libs, setLibs] = useState<ModelLibrary[]>([]);
  const [library, setLibrary] = useState<string | null>(null);
  const [picking, setPicking] = useState(false);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);

  const search = useCallback(
    async (q: string) => {
      if (!client) return;
      setSearching(true);
      setError(null);
      try {
        setResults((await client.hubSearch(q)).results);
        setOpen(null);
      } catch (e) {
        setError(describeError(e));
      } finally {
        setSearching(false);
      }
    },
    [client],
  );

  const pollDownloads = useCallback(async () => {
    if (!client) return;
    try {
      const d = (await client.hubDownloads()).downloads.filter((x) => x.repo !== 'move');
      setDownloads(d);
      if (d.some((x) => x.state === 'downloading')) timer.current = setTimeout(pollDownloads, 1000);
    } catch {
      timer.current = setTimeout(pollDownloads, 3000);
    }
  }, [client]);

  useEffect(() => {
    client
      ?.libraries()
      .then((r) => setLibs(r.libraries))
      .catch(() => undefined);
  }, [client]);

  useEffect(() => {
    search('coder');
    pollDownloads();
    return () => {
      if (timer.current) clearTimeout(timer.current);
    };
  }, [search, pollDownloads]);

  if (!client) return null;

  const target = libs.find((l) => l.path === library) ?? libs.find((l) => l.default);

  const start = async (repo: string, file: string) => {
    try {
      await client.hubDownload(repo, file, target?.path);
      if (timer.current) clearTimeout(timer.current);
      pollDownloads();
    } catch (e) {
      setError(describeError(e));
    }
  };

  return (
    <ScrollView style={styles.flex} contentContainerStyle={[styles.content, page]} keyboardShouldPersistTaps="handled">
      <Text style={[type.bodyMedium, styles.intro]}>
        Search Hugging Face for GGUF models. Downloads show up in Choose model when they finish.
      </Text>
      {target ? (
        <Pressable
          onPress={() => libs.length > 1 && setPicking(true)}
          disabled={libs.length < 2}
          style={({ hovered }: { pressed: boolean; hovered?: boolean }) => [styles.saveTo, hovered && styles.hover]}
          accessibilityRole="button"
          accessibilityLabel="Choose where downloads are saved"
        >
          <Icon name="folder-download-outline" size={20} color={m3.onSurfaceVariant} />
          <View style={styles.grow}>
            <Text style={type.bodySmall}>Saving to</Text>
            <Text style={type.bodyMedium} numberOfLines={1} ellipsizeMode="middle">
              {target.path}
              {target.disk ? ` · ${formatBytes(target.disk.free_bytes)} free` : ''}
            </Text>
          </View>
          {libs.length > 1 ? <Text style={[type.labelLarge, { color: m3.primary }]}>Change</Text> : null}
        </Pressable>
      ) : null}
      <Sheet visible={picking} onClose={() => setPicking(false)} title="Save downloads to">
        {libs.map((l) => (
          <Pressable
            key={l.path}
            onPress={() => {
              setLibrary(l.path);
              setPicking(false);
            }}
            style={({ hovered }: { pressed: boolean; hovered?: boolean }) => [styles.saveTo, hovered && styles.hover]}
            accessibilityRole="button"
          >
            <Icon
              name={l.path === target?.path ? 'radiobox-marked' : 'radiobox-blank'}
              size={20}
              color={l.path === target?.path ? m3.primary : m3.onSurfaceVariant}
            />
            <View style={styles.grow}>
              <Text style={type.bodyLarge} numberOfLines={2}>
                {l.path}
              </Text>
              <Text style={type.bodySmall}>
                {l.disk ? `${formatBytes(l.disk.free_bytes)} free` : 'Drive not available'}
                {l.disk?.kind === 'network' ? ' · network drive, slow to load' : ''}
              </Text>
            </View>
          </Pressable>
        ))}
      </Sheet>
      <Field
        label="Search"
        icon="magnify"
        placeholder="e.g. Qwen3 Coder"
        value={query}
        onChangeText={setQuery}
        onSubmitEditing={() => search(query.trim())}
        returnKeyType="search"
      />
      <View style={styles.suggest}>
        {SUGGESTIONS.map((s) => (
          <Pressable
            key={s}
            onPress={() => {
              setQuery(s);
              search(s);
            }}
            style={({ hovered }: { pressed: boolean; hovered?: boolean }) => [styles.chip, hovered && styles.hover]}
            accessibilityRole="button"
          >
            <Text style={[type.labelLarge, { color: m3.onSurfaceVariant }]}>{s}</Text>
          </Pressable>
        ))}
      </View>

      {error ? <Banner message={error} /> : null}

      {downloads.length ? (
        <>
          <Text style={[type.titleSmall, styles.section]}>Downloads</Text>
          <View style={styles.group}>
            {downloads.map((d) => (
              <DownloadRow key={d.id} d={d} onCancel={() => client.hubCancel(d.id).then(pollDownloads)} />
            ))}
          </View>
        </>
      ) : null}

      <Text style={[type.titleSmall, styles.section]}>Models</Text>
      {searching && !results ? <ActivityIndicator color={m3.primary} style={{ margin: space.lg }} /> : null}
      {results && results.length === 0 ? (
        <Text style={[type.bodyMedium, styles.intro]}>No GGUF models found.</Text>
      ) : null}
      <View style={styles.group}>
        {(results ?? []).map((r) => (
          <RepoRow
            key={r.id}
            repo={r}
            open={open === r.id}
            onToggle={() => setOpen((o) => (o === r.id ? null : r.id))}
            onDownload={(file) => start(r.id, file)}
            downloading={downloads.filter((d) => d.repo === r.id && d.state === 'downloading').map((d) => d.file)}
          />
        ))}
      </View>
    </ScrollView>
  );
}

function DownloadRow({ d, onCancel }: { d: HubDownload; onCancel: () => void }) {
  const pct = d.total_bytes ? Math.min(1, d.done_bytes / d.total_bytes) : 0;
  const name = d.file.split('/').pop() ?? d.file;
  const status =
    d.state === 'downloading'
      ? `${formatBytes(d.done_bytes)} of ${formatBytes(d.total_bytes)} · ${Math.round(pct * 100)}%`
      : d.state === 'done'
        ? d.repo === 'this PC'
          ? 'Copied into Relay. If it was the selected model, Relay now uses the copy.'
          : 'Downloaded. Pick it in Choose model.'
        : d.state === 'cancelled'
          ? 'Cancelled'
          : `Failed: ${d.error ?? 'unknown error'}`;
  return (
    <View style={styles.row}>
      <Icon
        name={
          d.state === 'done' ? 'check-circle-outline' : d.state === 'downloading' ? 'download' : 'alert-circle-outline'
        }
        size={22}
        color={d.state === 'done' ? m3.success : d.state === 'failed' ? m3.error : m3.onSurfaceVariant}
      />
      <View style={styles.grow}>
        <Text style={type.bodyLarge} numberOfLines={1} ellipsizeMode="middle">
          {name}
        </Text>
        <Text style={[type.bodySmall, d.state === 'failed' && { color: m3.error }]} numberOfLines={2}>
          {status}
        </Text>
        {d.state === 'downloading' ? (
          <View style={styles.track}>
            <View style={[styles.fill, { width: `${Math.round(pct * 100)}%` }]} />
          </View>
        ) : null}
      </View>
      {d.state === 'downloading' ? (
        <IconButton icon="close" label={`Cancel ${name}`} onPress={onCancel} />
      ) : d.state === 'done' ? (
        <Button title="Use" variant="text" onPress={() => router.push('/models')} />
      ) : null}
    </View>
  );
}

function RepoRow({
  repo,
  open,
  onToggle,
  onDownload,
  downloading,
}: {
  repo: HubRepo;
  open: boolean;
  onToggle: () => void;
  onDownload: (file: string) => void;
  downloading: string[];
}) {
  const { client } = useConnection();
  const [files, setFiles] = useState<{ files: HubFile[]; vram_mb: number } | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [owner, name] = repo.id.split('/');

  useEffect(() => {
    if (!open || files || !client) return;
    client
      .hubFiles(repo.id)
      .then(setFiles)
      .catch((e) => setError(describeError(e)));
  }, [open, files, client, repo.id]);

  return (
    <View>
      <Pressable
        onPress={onToggle}
        style={({ hovered, pressed }: { pressed: boolean; hovered?: boolean }) => [
          styles.row,
          (hovered || pressed) && styles.hover,
        ]}
        accessibilityRole="button"
        accessibilityState={{ expanded: open }}
        accessibilityLabel={repo.id}
      >
        <Icon name="cube-outline" size={22} color={m3.onSurfaceVariant} />
        <View style={styles.grow}>
          <Text style={type.bodyLarge} numberOfLines={1}>
            {name}
          </Text>
          <Text style={type.bodySmall} numberOfLines={1}>
            {owner} · {compact(repo.downloads)} downloads · {compact(repo.likes)} likes
          </Text>
        </View>
        <Icon name={open ? 'chevron-up' : 'chevron-down'} size={20} color={m3.outline} />
      </Pressable>
      {open ? (
        <View style={styles.files}>
          {error ? <Banner message={error} /> : null}
          {!files && !error ? <ActivityIndicator color={m3.primary} style={{ margin: space.md }} /> : null}
          {files && files.files.length === 0 ? (
            <Text style={[type.bodyMedium, { padding: space.md }]}>No GGUF files in this repository.</Text>
          ) : null}
          {files?.files.map((f) => {
            const fits = fit(f.size_bytes, files.vram_mb);
            const busy = downloading.includes(f.file);
            return (
              <View key={f.file} style={styles.fileRow}>
                <View style={styles.grow}>
                  <Text style={[type.labelLarge, { color: m3.onSurface }]} numberOfLines={1} ellipsizeMode="middle">
                    {f.quant ?? f.file.split('/').pop()}
                  </Text>
                  <Text style={type.bodySmall} numberOfLines={1}>
                    {formatBytes(f.size_bytes)}
                    {f.parts.length > 1 ? ` · ${f.parts.length} parts` : ''}
                    {fits ? ' · ' : ''}
                    {fits ? <Text style={{ color: fits.color }}>{fits.label}</Text> : null}
                  </Text>
                </View>
                <Button
                  title={busy ? 'Downloading' : 'Download'}
                  icon="download"
                  variant="tonal"
                  disabled={busy}
                  onPress={() => onDownload(f.file)}
                />
              </View>
            );
          })}
        </View>
      ) : null}
    </View>
  );
}

const styles = StyleSheet.create({
  saveTo: {
    flexDirection: 'row',
    alignItems: 'center',
    gap: space.md,
    paddingHorizontal: space.md,
    paddingVertical: space.sm,
    borderRadius: radius.md,
  },
  flex: { flex: 1, backgroundColor: m3.surface },
  grow: { flex: 1, minWidth: 0 },
  content: { padding: space.lg, paddingBottom: 64 },
  intro: { marginBottom: space.lg, marginHorizontal: space.xs },
  suggest: { flexDirection: 'row', flexWrap: 'wrap', gap: space.sm, marginBottom: space.md },
  chip: {
    height: 32,
    justifyContent: 'center',
    paddingHorizontal: space.md,
    borderRadius: radius.sm,
    borderWidth: 1,
    borderColor: m3.outlineVariant,
  },
  hover: { backgroundColor: m3.hover },
  section: { color: m3.onSurfaceVariant, marginTop: space.lg, marginBottom: space.sm, marginLeft: space.xs },
  group: { backgroundColor: m3.surfaceContainerLow, borderRadius: radius.xl, overflow: 'hidden' },
  row: {
    flexDirection: 'row',
    alignItems: 'center',
    gap: space.lg,
    minHeight: 64,
    paddingHorizontal: space.lg,
    paddingVertical: space.sm,
  },
  files: { backgroundColor: m3.surfaceContainer, paddingVertical: space.xs },
  fileRow: {
    flexDirection: 'row',
    alignItems: 'center',
    gap: space.md,
    paddingLeft: 62,
    paddingRight: space.lg,
    paddingVertical: space.sm,
  },
  track: { height: 4, borderRadius: 2, backgroundColor: m3.surfaceContainerHighest, marginTop: space.sm },
  fill: { height: 4, borderRadius: 2, backgroundColor: m3.primary },
});
