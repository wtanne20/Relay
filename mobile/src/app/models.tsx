import { router } from 'expo-router';
import { useCallback, useEffect, useState } from 'react';
import { ActivityIndicator, FlatList, Pressable, RefreshControl, StyleSheet, Text, View } from 'react-native';

import { colors, radius, space, touch } from '../components/theme';
import { Banner, EmptyState } from '../components/ui';
import { describeError } from '../lib/api';
import { useConnection } from '../lib/connection';
import { formatBytes } from '../lib/format';
import type { ModelFile } from '../lib/types';

/** GGUF models found on the PC (LM Studio's folder and Relay's models folder). */
export default function ModelsScreen() {
  const { client } = useConnection();
  const [data, setData] = useState<{ models: ModelFile[]; current: string; dirs: string[]; vram_mb: number } | null>(
    null,
  );
  const [error, setError] = useState<string | null>(null);
  const [refreshing, setRefreshing] = useState(false);
  const [busy, setBusy] = useState<string | null>(null);

  const load = useCallback(async () => {
    if (!client) return;
    try {
      setData(await client.models());
      setError(null);
    } catch (e) {
      setError(describeError(e));
    }
  }, [client]);

  useEffect(() => {
    load();
  }, [load]);

  if (!client) return null;

  const choose = async (m: ModelFile) => {
    if (m.path === data?.current) {
      router.back();
      return;
    }
    setBusy(m.path);
    try {
      await client.patchSettings({ llama: { model_path: m.path } });
      router.back();
    } catch (e) {
      setError(describeError(e));
    } finally {
      setBusy(null);
    }
  };

  const budget = data ? Math.max(0, data.vram_mb - 2560) * 1024 * 1024 : 0;

  return (
    <FlatList
      style={styles.flex}
      contentContainerStyle={styles.content}
      data={data?.models ?? []}
      keyExtractor={(m) => m.path}
      refreshControl={
        <RefreshControl
          refreshing={refreshing}
          onRefresh={async () => {
            setRefreshing(true);
            await load();
            setRefreshing(false);
          }}
          tintColor={colors.textMuted}
        />
      }
      ListHeaderComponent={
        <>
          {error ? <Banner message={error} actionLabel="Retry" onAction={load} /> : null}
          <Text style={styles.intro}>
            Picking a model reloads llama-server on the PC. Download more models in LM Studio, or copy .gguf files into
            Relay's models folder.
          </Text>
          {!data && !error ? <ActivityIndicator color={colors.accent} /> : null}
        </>
      }
      ListEmptyComponent={
        data ? <EmptyState title="No models found" body={`Relay looked in:\n${data.dirs.join('\n')}`} /> : undefined
      }
      renderItem={({ item }) => {
        const current = item.path === data?.current;
        const tooBig = budget > 0 && item.size_bytes > budget;
        return (
          <Pressable
            onPress={() => choose(item)}
            disabled={busy !== null}
            style={({ pressed }) => [styles.item, current && styles.itemCurrent, pressed && styles.pressed]}
            accessibilityRole="button"
            accessibilityState={{ selected: current }}
            accessibilityLabel={`${item.name}${current ? ', current model' : ''}`}
          >
            <View style={styles.grow}>
              <Text style={styles.name} numberOfLines={2}>
                {item.name}
              </Text>
              <Text style={styles.meta} numberOfLines={1} ellipsizeMode="middle">
                {formatBytes(item.size_bytes)}
                {tooBig ? ' · may not fit in VRAM' : ''} · {item.path}
              </Text>
            </View>
            {busy === item.path ? (
              <ActivityIndicator color={colors.accent} />
            ) : current ? (
              <Text style={styles.check}>✓</Text>
            ) : null}
          </Pressable>
        );
      }}
    />
  );
}

const styles = StyleSheet.create({
  flex: { flex: 1, backgroundColor: colors.bg },
  grow: { flex: 1 },
  content: { padding: space.lg, gap: space.sm },
  intro: { color: colors.textMuted, fontSize: 14, lineHeight: 20, marginBottom: space.md },
  item: {
    flexDirection: 'row',
    alignItems: 'center',
    gap: space.md,
    minHeight: touch + 16,
    padding: space.md,
    borderRadius: radius.md,
    backgroundColor: colors.surface,
    borderWidth: 1,
    borderColor: colors.border,
  },
  itemCurrent: { borderColor: colors.accent },
  pressed: { backgroundColor: colors.surfaceRaised },
  name: { color: colors.text, fontSize: 16, fontWeight: '500' },
  meta: { color: colors.textMuted, fontSize: 12, marginTop: 2 },
  check: { color: colors.accent, fontSize: 20, fontWeight: '700' },
});
