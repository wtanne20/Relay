import { useCallback, useEffect, useState } from 'react';
import { ActivityIndicator, RefreshControl, ScrollView, StyleSheet, Text, View } from 'react-native';

import { modelLabel } from '../components/ModelPicker';
import { Group, SectionTitle } from '../components/SettingsControls';
import { m3, radius, space, type } from '../components/theme';
import { Banner, Dot, Icon, type IconName } from '../components/ui';
import { describeError } from '../lib/api';
import { useConnection } from '../lib/connection';
import { relativeTime } from '../lib/format';
import { usePageStyle } from '../lib/layout';
import type { MemoryUse, StatsResponse } from '../lib/types';

const STATE_LABEL: Record<StatsResponse['brain']['model']['state'], string> = {
  idle: 'Unloaded',
  starting: 'Loading',
  ready: 'Loaded',
  error: 'Error',
};

function gb(mb: number): string {
  return mb >= 1024 ? `${(mb / 1024).toFixed(1)} GB` : `${Math.round(mb)} MB`;
}

function speed(tps: number): string {
  return `${tps >= 10 ? Math.round(tps) : tps.toFixed(1)} tok/s`;
}

/** A labelled bar: used of total, with an optional note. */
function Meter({ icon, label, used, total, note }: { icon: IconName; label: string; used: number; total: number; note?: string }) {
  const frac = total > 0 ? Math.min(1, used / total) : 0;
  const color = frac > 0.9 ? m3.error : frac > 0.75 ? m3.warning : m3.primary;
  return (
    <View style={styles.meter} accessibilityLabel={`${label}: ${gb(used)} of ${gb(total)} used`}>
      <View style={styles.meterHead}>
        <Icon name={icon} size={18} color={m3.onSurfaceVariant} />
        <Text style={[type.bodyLarge, styles.grow]} numberOfLines={1}>
          {label}
        </Text>
        <Text style={type.bodyMedium}>
          {gb(used)} / {gb(total)}
        </Text>
      </View>
      <View style={styles.track}>
        <View style={[styles.fill, { width: `${Math.round(frac * 100)}%`, backgroundColor: color }]} />
      </View>
      {note ? <Text style={[type.bodySmall, styles.muted]}>{note}</Text> : null}
    </View>
  );
}

function RamMeter({ memory, unknown }: { memory: MemoryUse | null; unknown: string }) {
  if (!memory) return <Text style={[type.bodyMedium, styles.pad, styles.muted]}>{unknown}</Text>;
  return (
    <Meter
      icon="memory"
      label="Memory"
      used={memory.used_mb}
      total={memory.total_mb}
      note={memory.model_mb != null ? `The model uses ${gb(memory.model_mb)}` : undefined}
    />
  );
}

/**
 * How fast each model writes, and how much memory the brain (and its GPU)
 * and every linked computer use. Refreshes every few seconds.
 */
export default function StatsScreen() {
  const page = usePageStyle();
  const { client } = useConnection();
  const [stats, setStats] = useState<StatsResponse | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [refreshing, setRefreshing] = useState(false);

  const load = useCallback(async () => {
    if (!client) return;
    try {
      setStats(await client.stats());
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
  if (!stats) {
    return (
      <View style={styles.center}>
        {error ? <Banner message={error} actionLabel="Retry" onAction={load} /> : <ActivityIndicator color={m3.primary} />}
      </View>
    );
  }

  const { brain, speeds, devices } = stats;
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
        />
      }
    >
      {error ? <Banner message={error} /> : null}

      <SectionTitle title="Speed" />
      <Group>
        {speeds.length === 0 ? (
          <Text style={[type.bodyMedium, styles.pad, styles.muted]}>
            No replies since Relay started. Send a message and the speed shows up here.
          </Text>
        ) : (
          speeds.map((s) => (
            <View key={s.model} style={styles.speedRow}>
              <View style={styles.grow}>
                <Text style={type.bodyLarge} numberOfLines={1}>
                  {modelLabel(s.model)}
                </Text>
                <Text style={[type.bodySmall, styles.muted]} numberOfLines={2}>
                  {s.replies} {s.replies === 1 ? 'reply' : 'replies'} · {s.tokens.toLocaleString()} tokens · last{' '}
                  {relativeTime(s.last_at)}
                  {s.last_prompt_per_sec ? ` · reads ${speed(s.last_prompt_per_sec)}` : ''}
                </Text>
              </View>
              <View style={styles.speedValue}>
                <Text style={[type.titleMedium, { color: m3.primary }]}>{speed(s.avg_tokens_per_sec)}</Text>
                <Text style={[type.bodySmall, styles.muted]}>last {speed(s.last_tokens_per_sec)}</Text>
              </View>
            </View>
          ))
        )}
      </Group>

      <SectionTitle title={`${brain.name} (brain)`} />
      <Group>
        <View style={styles.modelRow}>
          <Dot color={brain.model.state === 'ready' ? m3.success : brain.model.state === 'error' ? m3.error : m3.outline} />
          <Text style={[type.bodyMedium, styles.grow]} numberOfLines={1}>
            {STATE_LABEL[brain.model.state]}
            {brain.model.file ? ` · ${modelLabel(brain.model.file)}` : ''}
          </Text>
        </View>
        <RamMeter memory={brain.memory} unknown="Memory use isn’t available on this computer." />
        {brain.gpus.map((g, i) => (
          <Meter key={`${g.name}${i}`} icon="expansion-card-variant" label={g.name} used={g.used_mb} total={g.total_mb} />
        ))}
      </Group>

      {devices.map((d) => (
        <View key={d.id}>
          <SectionTitle title={d.name} />
          <Group>
            <View style={styles.modelRow}>
              <Dot color={d.connected ? m3.success : m3.outline} />
              <Text style={[type.bodyMedium, styles.grow]}>{d.connected ? 'Online' : 'Offline'}</Text>
            </View>
            {d.connected ? (
              <RamMeter memory={d.memory} unknown="Update Relay on this computer to see its memory use." />
            ) : null}
          </Group>
        </View>
      ))}

      <Text style={[type.bodySmall, styles.footnote]}>
        Speeds count since Relay last started. Linked computers run tools, not the model, so only the brain has
        speeds.
      </Text>
    </ScrollView>
  );
}

const styles = StyleSheet.create({
  flex: { flex: 1, backgroundColor: m3.surface },
  content: { padding: space.lg, paddingBottom: 64 },
  center: { flex: 1, alignItems: 'center', justifyContent: 'center', padding: space.lg, backgroundColor: m3.surface },
  grow: { flex: 1, minWidth: 0 },
  muted: { color: m3.onSurfaceVariant },
  pad: { padding: space.lg },
  speedRow: {
    flexDirection: 'row',
    alignItems: 'center',
    gap: space.md,
    minHeight: 64,
    paddingHorizontal: space.lg,
    paddingVertical: space.sm,
  },
  speedValue: { alignItems: 'flex-end' },
  modelRow: { flexDirection: 'row', alignItems: 'center', gap: space.sm, paddingHorizontal: space.lg, paddingTop: space.md },
  meter: { paddingHorizontal: space.lg, paddingVertical: space.md, gap: 6 },
  meterHead: { flexDirection: 'row', alignItems: 'center', gap: space.sm },
  track: { height: 8, borderRadius: radius.pill, backgroundColor: m3.surfaceContainerHighest, overflow: 'hidden' },
  fill: { height: 8, borderRadius: radius.pill },
  footnote: { color: m3.onSurfaceVariant, marginTop: space.lg, paddingHorizontal: space.sm, lineHeight: 18 },
});
