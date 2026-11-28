import { useCallback, useEffect, useRef, useState } from 'react';
import { ActivityIndicator, StyleSheet, Text, View } from 'react-native';

import { describeError, type RelayClient } from '../lib/api';
import { formatBytes, relativeTime } from '../lib/format';
import type { HubDownload, ModelUpdate, ModelUpdates } from '../lib/types';
import { alert } from './Dialogs';
import { m3, radius, space, type } from './theme';
import { Button, Icon } from './ui';

/**
 * The last model update check, polled while an update downloads. `reload`
 * fetches it again; `check` asks Hugging Face now.
 */
export function useModelUpdates(client: RelayClient | null | undefined) {
  const [data, setData] = useState<ModelUpdates | null>(null);
  const [downloads, setDownloads] = useState<HubDownload[]>([]);
  const [checking, setChecking] = useState(false);
  const alive = useRef(true);

  const reload = useCallback(async () => {
    if (!client) return;
    try {
      const d = await client.modelUpdates();
      if (!alive.current) return;
      setData(d);
      if (d.updates.some((u) => u.updating)) {
        const h = await client.hubDownloads();
        if (alive.current) setDownloads(h.downloads.filter((x) => x.group === 'update'));
      }
    } catch {
      // Older servers don't have update checks: show nothing.
    }
  }, [client]);

  const check = useCallback(async () => {
    if (!client) return;
    setChecking(true);
    try {
      const d = await client.modelUpdates(true);
      if (alive.current) setData(d);
    } catch (e) {
      alert("Couldn't check for updates", describeError(e));
    } finally {
      if (alive.current) setChecking(false);
    }
  }, [client]);

  useEffect(() => {
    alive.current = true;
    reload();
    return () => {
      alive.current = false;
    };
  }, [reload]);

  const busy = !!data?.updates.some((u) => u.updating);
  useEffect(() => {
    if (!busy) return;
    const t = setInterval(reload, 2000);
    return () => clearInterval(t);
  }, [busy, reload]);

  return { data, downloads, checking: checking || !!data?.checking, reload, check, setData };
}

type Hook = ReturnType<typeof useModelUpdates>;

/** True when `a` and `b` name the same file on the PC. */
export function samePath(a: string, b: string) {
  const n = (p: string) => p.replace(/\\/g, '/').toLowerCase();
  return n(a) === n(b);
}

/**
 * The updates for one kind of model, with Update and Skip, plus when Relay
 * last checked and a Check now button. `onUpdated` runs once a new version
 * is in.
 */
export function ModelUpdatesCard({
  client,
  kind,
  updates: u,
  onUpdated,
}: {
  client: RelayClient;
  kind: 'chat' | 'image';
  updates: Hook;
  onUpdated?: () => void;
}) {
  const [starting, setStarting] = useState<string | null>(null);
  const list = (u.data?.updates ?? []).filter((x) => x.kind === kind);
  const count = useRef(list.length);
  const updatingCount = list.filter((x) => x.updating).length;
  const wasUpdating = useRef(updatingCount);
  useEffect(() => {
    // One finished: the model list changed (new size).
    if (updatingCount < wasUpdating.current || list.length < count.current) onUpdated?.();
    wasUpdating.current = updatingCount;
    count.current = list.length;
  }, [updatingCount, list.length, onUpdated]);

  if (!u.data) return null;

  const update = (m: ModelUpdate) =>
    alert(
      `Update ${m.name}?`,
      `Relay downloads the new version (${formatBytes(m.size_bytes)}) next to the old one, then swaps it in once nothing is using the model. Chats keep using it as before.`,
      [
        { text: 'Cancel', style: 'cancel' },
        {
          text: 'Update',
          onPress: async () => {
            setStarting(m.path);
            try {
              await client.applyModelUpdate(m.path);
              await u.reload();
            } catch (e) {
              alert("Couldn't update", describeError(e));
            } finally {
              setStarting(null);
            }
          },
        },
      ],
    );

  const skip = async (m: ModelUpdate) => {
    try {
      u.setData(await client.skipModelUpdate(m.path));
    } catch (e) {
      alert("Couldn't skip it", describeError(e));
    }
  };

  const checked = u.data.checked_at ? `Checked for updates ${relativeTime(u.data.checked_at)}` : 'Not checked for updates yet';

  return (
    <View style={styles.wrap}>
      {list.map((m) => {
        const d = u.downloads.find((x) => x.repo === m.repo && x.file === m.name && x.state === 'downloading');
        const pct = d && d.total_bytes > 0 ? Math.round((d.done_bytes / d.total_bytes) * 100) : null;
        return (
          <View key={m.path} style={styles.card}>
            <View style={styles.head}>
              <Icon name="update" size={22} color={m3.onTertiaryContainer} />
              <View style={styles.grow}>
                <Text style={[type.titleSmall, styles.fg]} numberOfLines={2}>
                  {m.name}
                </Text>
                <Text style={[type.bodyMedium, styles.fg]}>
                  {m.updating
                    ? pct != null && pct < 100
                      ? `Downloading the new version… ${pct}%`
                      : 'Swapping in the new version once the model is free…'
                    : `A newer version is on Hugging Face (${m.repo}). ${formatBytes(m.size_bytes)} to download.`}
                </Text>
              </View>
            </View>
            {m.updating ? (
              <View style={styles.track}>
                <View style={[styles.fill, { width: `${pct ?? 100}%` }]} />
              </View>
            ) : (
              <View style={styles.actions}>
                <Button title="Skip this version" variant="text" onPress={() => skip(m)} />
                <Button
                  title="Update"
                  icon="download-outline"
                  loading={starting === m.path}
                  disabled={starting !== null}
                  onPress={() => update(m)}
                />
              </View>
            )}
          </View>
        );
      })}
      <View style={styles.checkRow}>
        <Text style={[type.bodySmall, styles.grow]}>{list.length ? checked : `${checked}. Your models are up to date.`}</Text>
        {u.checking ? (
          <ActivityIndicator color={m3.primary} />
        ) : (
          <Button title="Check now" variant="text" icon="refresh" onPress={u.check} />
        )}
      </View>
    </View>
  );
}

/** A small "Update available" chip for a model row. */
export function UpdateChip() {
  return (
    <View style={styles.chip}>
      <Text style={[type.labelMedium, { color: m3.onTertiaryContainer }]}>Update available</Text>
    </View>
  );
}

const styles = StyleSheet.create({
  wrap: { gap: space.sm, marginBottom: space.md },
  card: {
    backgroundColor: m3.tertiaryContainer,
    borderRadius: radius.xl,
    padding: space.lg,
    gap: space.sm,
  },
  head: { flexDirection: 'row', gap: space.md, alignItems: 'flex-start' },
  grow: { flex: 1, minWidth: 0 },
  fg: { color: m3.onTertiaryContainer },
  actions: { flexDirection: 'row', justifyContent: 'flex-end', gap: space.sm, flexWrap: 'wrap' },
  track: { height: 4, borderRadius: 2, backgroundColor: m3.surfaceContainerHighest, overflow: 'hidden' },
  fill: { height: 4, backgroundColor: m3.primary },
  checkRow: { flexDirection: 'row', alignItems: 'center', gap: space.sm, marginHorizontal: space.xs },
  chip: {
    backgroundColor: m3.tertiaryContainer,
    borderRadius: radius.sm,
    paddingHorizontal: space.sm,
    paddingVertical: 2,
  },
});
