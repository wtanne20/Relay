import { useCallback, useEffect, useRef, useState } from 'react';
import { ActivityIndicator, Pressable, ScrollView, StyleSheet, Text, TextInput, View } from 'react-native';

import { alert } from './Dialogs';
import { Group, NavRow, SectionTitle } from './SettingsControls';
import { Sheet } from './Sheet';
import { m3, radius, space, type } from './theme';
import { Banner, Button, Icon, IconButton } from './ui';
import { describeError, type RelayClient } from '../lib/api';
import { formatBytes } from '../lib/format';
import { imagesApi, isImageDownload, type ImageHubFile, type Starter } from '../lib/images';
import type { HubDownload, HubRepo } from '../lib/types';

const FOLDER_LABEL: Record<string, string> = {
  diffusion_models: 'Main model',
  checkpoints: 'Checkpoint',
  text_encoders: 'Text encoder',
  vae: 'VAE',
  loras: 'LoRA',
};

const progressOf = (ds: HubDownload[]) => {
  const total = ds.reduce((a, d) => a + d.total_bytes, 0);
  const done = ds.reduce((a, d) => a + d.done_bytes, 0);
  return { total, done, pct: total ? Math.min(1, done / total) : 0 };
};

/**
 * Image model downloads: starter models (every file, picked for the GPU,
 * set up when they're in) and any file from Hugging Face.
 *
 * `onChanged` runs when a download finishes, so the screen can rescan.
 */
export function ImageDownloads({ client, onChanged }: { client: RelayClient; onChanged: () => void }) {
  const api = imagesApi(client);
  const [starters, setStarters] = useState<Starter[] | null>(null);
  const [vram, setVram] = useState(0);
  const [downloads, setDownloads] = useState<HubDownload[]>([]);
  const [busy, setBusy] = useState<string | null>(null);
  const [searching, setSearching] = useState(false);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const active = useRef(0);
  const onChangedRef = useRef(onChanged);
  onChangedRef.current = onChanged;

  const poll = useCallback(async () => {
    if (timer.current) clearTimeout(timer.current);
    try {
      const [s, d] = await Promise.all([api.starters(), client.hubDownloads()]);
      setStarters(s.starters);
      setVram(s.vram_mb);
      const mine = d.downloads.filter(isImageDownload);
      setDownloads(mine);
      const running = mine.filter((x) => x.state === 'downloading').length;
      // One finished: the new files (and maybe a new model) show up.
      if (running < active.current) setTimeout(() => onChangedRef.current(), 2500);
      active.current = running;
      if (running || s.starters.some((x) => x.downloading)) timer.current = setTimeout(poll, 1000);
    } catch {
      timer.current = setTimeout(poll, 4000);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [client]);

  useEffect(() => {
    poll();
    return () => {
      if (timer.current) clearTimeout(timer.current);
    };
  }, [poll]);

  const start = async (s: Starter) => {
    setBusy(s.id);
    try {
      await api.downloadStarter(s.id);
      await poll();
    } catch (e) {
      alert(`Couldn't download ${s.name}`, describeError(e));
    } finally {
      setBusy(null);
    }
  };

  const cancelGroup = (group: string) =>
    Promise.all(
      downloads.filter((d) => d.group === group && d.state === 'downloading').map((d) => client.hubCancel(d.id)),
    ).then(poll, poll);

  const singles = downloads.filter((d) => d.group === 'image-file');

  return (
    <>
      <SectionTitle title="Download a model" />
      <Group>
        {starters === null ? (
          <View style={styles.loading}>
            <ActivityIndicator color={m3.primary} />
          </View>
        ) : (
          starters.map((s) => (
            <StarterRow
              key={s.id}
              s={s}
              vram={vram}
              downloads={downloads.filter((d) => d.group === `image:${s.id}`)}
              busy={busy === s.id}
              onDownload={() => start(s)}
              onCancel={() => cancelGroup(`image:${s.id}`)}
            />
          ))
        )}
        <NavRow
          icon="magnify"
          label="Search Hugging Face"
          value="Any model file, saved in the folder it's for"
          onPress={() => setSearching(true)}
        />
      </Group>
      {singles.length ? (
        <Group>
          {singles.slice(0, 6).map((d) => (
            <FileDownloadRow key={d.id} d={d} onCancel={() => client.hubCancel(d.id).then(poll, poll)} />
          ))}
        </Group>
      ) : null}
      <HubSheet
        visible={searching}
        client={client}
        downloads={downloads}
        onClose={() => setSearching(false)}
        onStarted={poll}
      />
    </>
  );
}

function StarterRow({
  s,
  vram,
  downloads,
  busy,
  onDownload,
  onCancel,
}: {
  s: Starter;
  vram: number;
  downloads: HubDownload[];
  busy: boolean;
  onDownload: () => void;
  onCancel: () => void;
}) {
  const running = downloads.filter((d) => d.state === 'downloading');
  const p = progressOf(running);
  // Bytes already on disk count as done.
  const have = s.total_bytes - s.missing_bytes;
  const pct = s.downloading ? Math.min(1, (have + p.done) / Math.max(1, have + p.total)) : 0;
  const gpu = vram ? `${Math.round(vram / 1024)} GB GPU` : 'GPU';
  const detail = s.set_up
    ? 'Downloaded and ready to use'
    : s.downloading
      ? `Downloading · ${formatBytes(have + p.done)} of ${formatBytes(have + p.total)} · ${Math.round(pct * 100)}%`
      : s.missing_bytes === 0
        ? 'Files downloaded, tap Add to set it up'
        : `${formatBytes(s.total_bytes)} · ${s.big ? 'sharper version' : 'smaller version'}, picked for your ${gpu}`;
  return (
    <View style={styles.starter}>
      <View style={[styles.starterIcon, s.set_up && { backgroundColor: m3.secondaryContainer }]}>
        <Icon name={s.edit ? 'image-edit-outline' : 'image-filter-hdr'} color={s.set_up ? m3.onSecondaryContainer : m3.primary} />
      </View>
      <View style={styles.grow}>
        <View style={styles.titleRow}>
          <Text style={type.bodyLarge} numberOfLines={1}>
            {s.name}
          </Text>
          {s.edit ? (
            <View style={styles.chip}>
              <Text style={[type.labelMedium, { color: m3.onTertiaryContainer }]}>Edits pictures</Text>
            </View>
          ) : null}
        </View>
        <Text style={[type.bodyMedium, styles.muted]} numberOfLines={2}>
          {s.blurb}
        </Text>
        <Text style={[type.bodySmall, styles.muted]} numberOfLines={2}>
          {detail}
        </Text>
        {s.downloading ? (
          <View style={styles.track}>
            <View style={[styles.fill, { width: `${Math.round(pct * 100)}%` }]} />
          </View>
        ) : null}
        {s.error && !s.downloading && !s.set_up ? (
          <Text style={[type.bodySmall, { color: m3.error }]} numberOfLines={3}>
            Last try failed: {s.error}
          </Text>
        ) : null}
      </View>
      {s.set_up ? (
        <Icon name="check-circle" color={m3.success} />
      ) : s.downloading ? (
        <IconButton icon="close" label={`Stop downloading ${s.name}`} onPress={onCancel} />
      ) : (
        <Button
          title={s.missing_bytes === 0 ? 'Add' : 'Download'}
          icon={s.missing_bytes === 0 ? 'plus' : 'download'}
          variant="tonal"
          loading={busy}
          onPress={onDownload}
        />
      )}
    </View>
  );
}

function FileDownloadRow({ d, onCancel }: { d: HubDownload; onCancel: () => void }) {
  const pct = d.total_bytes ? Math.min(1, d.done_bytes / d.total_bytes) : 0;
  const status =
    d.state === 'downloading'
      ? `${formatBytes(d.done_bytes)} of ${formatBytes(d.total_bytes)} · ${Math.round(pct * 100)}%`
      : d.state === 'done'
        ? 'Downloaded. Set it up with Set up a model.'
        : d.state === 'cancelled'
          ? 'Cancelled'
          : `Failed: ${d.error ?? 'unknown error'}`;
  return (
    <View style={styles.starter}>
      <Icon
        name={d.state === 'done' ? 'check-circle-outline' : d.state === 'downloading' ? 'download' : 'alert-circle-outline'}
        color={d.state === 'done' ? m3.success : d.state === 'failed' ? m3.error : m3.onSurfaceVariant}
      />
      <View style={styles.grow}>
        <Text style={type.bodyLarge} numberOfLines={1} ellipsizeMode="middle">
          {d.file}
        </Text>
        <Text style={[type.bodySmall, d.state === 'failed' ? { color: m3.error } : styles.muted]} numberOfLines={2}>
          {status}
        </Text>
        {d.state === 'downloading' ? (
          <View style={styles.track}>
            <View style={[styles.fill, { width: `${Math.round(pct * 100)}%` }]} />
          </View>
        ) : null}
      </View>
      {d.state === 'downloading' ? <IconButton icon="close" label={`Cancel ${d.file}`} onPress={onCancel} /> : null}
    </View>
  );
}

const compact = (n: number) => (n >= 1e6 ? `${(n / 1e6).toFixed(1)}M` : n >= 1e3 ? `${Math.round(n / 1e3)}k` : String(n));

/** Search Hugging Face, open a repository, download a file into a folder. */
function HubSheet({
  visible,
  client,
  downloads,
  onClose,
  onStarted,
}: {
  visible: boolean;
  client: RelayClient;
  downloads: HubDownload[];
  onClose: () => void;
  onStarted: () => void;
}) {
  const api = imagesApi(client);
  const [query, setQuery] = useState('');
  const [results, setResults] = useState<HubRepo[] | null>(null);
  const [repo, setRepo] = useState<string | null>(null);
  const [files, setFiles] = useState<ImageHubFile[] | null>(null);
  const [folders, setFolders] = useState<Record<string, string>>({});
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [started, setStarted] = useState<Record<string, boolean>>({});

  useEffect(() => {
    if (visible) setError(null);
  }, [visible]);

  const search = async (q: string) => {
    if (!q.trim()) return;
    setLoading(true);
    setError(null);
    setRepo(null);
    try {
      setResults((await api.hubSearch(q.trim())).results);
    } catch (e) {
      setError(describeError(e));
    } finally {
      setLoading(false);
    }
  };

  const openRepo = async (id: string) => {
    setRepo(id);
    setFiles(null);
    setLoading(true);
    setError(null);
    try {
      const r = await api.hubFiles(id);
      setFiles(r.files);
      setFolders(Object.fromEntries(r.files.map((f) => [f.file, f.category])));
    } catch (e) {
      setError(describeError(e));
    } finally {
      setLoading(false);
    }
  };

  const download = async (f: ImageHubFile) => {
    if (!repo) return;
    try {
      await api.hubDownload(repo, f.file, folders[f.file] ?? f.category);
      setStarted((s) => ({ ...s, [`${repo}/${f.file}`]: true }));
      onStarted();
    } catch (e) {
      setError(describeError(e));
    }
  };

  return (
    <Sheet visible={visible} onClose={onClose} title={repo ? repo.split('/').pop() ?? repo : 'Search Hugging Face'}>
      {repo ? (
        <Pressable onPress={() => setRepo(null)} style={styles.back} accessibilityRole="button">
          <Icon name="arrow-left" size={18} color={m3.primary} />
          <Text style={[type.labelLarge, { color: m3.primary }]}>All results</Text>
        </Pressable>
      ) : (
        <View style={styles.search}>
          <Icon name="magnify" size={20} />
          <TextInput
            value={query}
            onChangeText={setQuery}
            onSubmitEditing={() => search(query)}
            placeholder="Model name, e.g. FLUX schnell GGUF"
            placeholderTextColor={m3.outline}
            style={styles.searchInput}
            returnKeyType="search"
            autoCorrect={false}
            autoCapitalize="none"
            autoFocus
          />
          {query ? <Button title="Search" variant="text" onPress={() => search(query)} /> : null}
        </View>
      )}
      {error ? <Banner message={error} /> : null}
      <ScrollView style={{ maxHeight: 480 }} keyboardShouldPersistTaps="handled">
        {loading ? (
          <View style={styles.loading}>
            <ActivityIndicator color={m3.primary} />
          </View>
        ) : repo ? (
          files?.length === 0 ? (
            <Text style={[type.bodyMedium, styles.muted]}>No model files here (.gguf or .safetensors).</Text>
          ) : (
            files?.map((f) => {
              const key = `${repo}/${f.file}`;
              const running = downloads.some((d) => d.repo === repo && d.file === f.name && d.state === 'downloading');
              const folder = folders[f.file] ?? f.category;
              return (
                <View key={f.file} style={styles.file}>
                  <View style={styles.fileHead}>
                    <View style={styles.grow}>
                      <Text style={type.bodyMedium} numberOfLines={2}>
                        {f.name}
                      </Text>
                      <Text style={[type.bodySmall, styles.muted]}>
                        {formatBytes(f.size_bytes)}
                        {f.quant ? ` · ${f.quant}` : ''}
                      </Text>
                    </View>
                    {f.have ? (
                      <Text style={[type.labelMedium, { color: m3.success }]}>Downloaded</Text>
                    ) : running || started[key] ? (
                      <Text style={[type.labelMedium, styles.muted]}>Downloading</Text>
                    ) : (
                      <Button title="Download" variant="tonal" icon="download" onPress={() => download(f)} />
                    )}
                  </View>
                  {f.have || running || started[key] ? null : (
                    <View style={styles.folders}>
                      <Text style={[type.labelMedium, styles.muted]}>Save as</Text>
                      {Object.keys(FOLDER_LABEL).map((c) => (
                        <Pressable
                          key={c}
                          onPress={() => setFolders((x) => ({ ...x, [f.file]: c }))}
                          style={[styles.folder, folder === c && styles.folderOn]}
                          accessibilityRole="radio"
                          accessibilityState={{ checked: folder === c }}
                        >
                          <Text style={[type.labelMedium, { color: folder === c ? m3.onSecondaryContainer : m3.onSurfaceVariant }]}>
                            {FOLDER_LABEL[c]}
                          </Text>
                        </Pressable>
                      ))}
                    </View>
                  )}
                </View>
              );
            })
          )
        ) : results === null ? (
          <Text style={[type.bodyMedium, styles.muted]}>
            Image models usually come in parts: the main model, a text encoder and a VAE. Each repository&apos;s page
            says which ones it needs. Relay guesses the folder for each file; change it if the guess is wrong.
          </Text>
        ) : results.length === 0 ? (
          <Text style={[type.bodyMedium, styles.muted]}>Nothing found.</Text>
        ) : (
          results.map((r) => (
            <Pressable
              key={r.id}
              onPress={() => openRepo(r.id)}
              style={({ pressed }) => [styles.result, pressed && { backgroundColor: m3.pressed }]}
              accessibilityRole="button"
            >
              <View style={styles.grow}>
                <Text style={type.bodyLarge} numberOfLines={1}>
                  {r.id.split('/').pop()}
                </Text>
                <Text style={[type.bodySmall, styles.muted]} numberOfLines={1}>
                  {r.id.split('/')[0]} · {compact(r.downloads)} downloads
                </Text>
              </View>
              <Icon name="chevron-right" size={20} />
            </Pressable>
          ))
        )}
      </ScrollView>
      <View style={styles.actions}>
        <Button title="Done" variant="text" onPress={onClose} />
      </View>
    </Sheet>
  );
}

const styles = StyleSheet.create({
  grow: { flex: 1, minWidth: 0 },
  muted: { color: m3.onSurfaceVariant },
  loading: { padding: space.lg, alignItems: 'center' },
  starter: {
    flexDirection: 'row',
    alignItems: 'center',
    gap: space.lg,
    paddingHorizontal: space.lg,
    paddingVertical: space.md,
  },
  starterIcon: {
    width: 44,
    height: 44,
    borderRadius: 14,
    backgroundColor: m3.surfaceContainerHigh,
    alignItems: 'center',
    justifyContent: 'center',
  },
  titleRow: { flexDirection: 'row', alignItems: 'center', gap: space.sm, flexWrap: 'wrap' },
  chip: { paddingHorizontal: 8, paddingVertical: 2, borderRadius: 8, backgroundColor: m3.tertiaryContainer },
  track: { height: 4, borderRadius: 2, backgroundColor: m3.surfaceContainerHighest, marginTop: space.sm, overflow: 'hidden' },
  fill: { height: 4, backgroundColor: m3.primary },
  search: {
    flexDirection: 'row',
    alignItems: 'center',
    gap: space.sm,
    height: 44,
    paddingLeft: space.md,
    borderRadius: 22,
    backgroundColor: m3.surfaceContainerHigh,
    marginBottom: space.sm,
  },
  searchInput: { flex: 1, color: m3.onSurface, fontSize: 15, height: 44 },
  back: { flexDirection: 'row', alignItems: 'center', gap: 6, marginBottom: space.sm, minHeight: 32 },
  result: { flexDirection: 'row', alignItems: 'center', gap: space.md, minHeight: 56, paddingHorizontal: space.sm, borderRadius: radius.md },
  file: { borderRadius: radius.md, backgroundColor: m3.surfaceContainerHigh, padding: space.md, marginBottom: space.sm, gap: space.sm },
  fileHead: { flexDirection: 'row', alignItems: 'center', gap: space.md },
  folders: { flexDirection: 'row', flexWrap: 'wrap', alignItems: 'center', gap: 6 },
  folder: { paddingHorizontal: 10, paddingVertical: 4, borderRadius: 8, borderWidth: 1, borderColor: m3.outlineVariant },
  folderOn: { backgroundColor: m3.secondaryContainer, borderColor: m3.secondaryContainer },
  actions: { flexDirection: 'row', justifyContent: 'flex-end', marginTop: space.md },
});
