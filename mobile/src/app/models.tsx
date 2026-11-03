import { router } from 'expo-router';
import { useCallback, useEffect, useState } from 'react';
import { ActivityIndicator, FlatList, Pressable, RefreshControl, StyleSheet, Text, View } from 'react-native';

import { m3, radius, space, type } from '../components/theme';
import { TagChips, TagsSheet } from '../components/ModelTags';
import { Banner, EmptyState, Icon, IconButton } from '../components/ui';
import { describeError } from '../lib/api';
import { usePageStyle } from '../lib/layout';
import { useConnection } from '../lib/connection';
import { formatBytes } from '../lib/format';
import type { ModelFile } from '../lib/types';

/** GGUF models found on the PC (LM Studio's folder and Relay's models folder). */
export default function ModelsScreen() {
  const page = usePageStyle();
  const { client } = useConnection();
  const [data, setData] = useState<{ models: ModelFile[]; current: string; dirs: string[]; vram_mb: number } | null>(
    null,
  );
  const [error, setError] = useState<string | null>(null);
  const [refreshing, setRefreshing] = useState(false);
  const [busy, setBusy] = useState<string | null>(null);
  const [tagging, setTagging] = useState<{ name: string; tags: string[]; custom: boolean } | null>(null);

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
    <>
      <FlatList
        style={styles.flex}
        contentContainerStyle={[styles.content, page]}
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
            tintColor={m3.onSurfaceVariant}
          />
        }
        ListHeaderComponent={
          <>
            {error ? <Banner message={error} actionLabel="Retry" onAction={load} /> : null}
            <Text style={[type.bodyMedium, styles.intro]}>
              Picking a model reloads llama-server on the PC. Download more models in LM Studio, or copy .gguf files
              into Relay&apos;s models folder.
            </Text>
            {!data && !error ? <ActivityIndicator color={m3.primary} /> : null}
          </>
        }
        ListEmptyComponent={
          data ? (
            <EmptyState
              icon="cube-off-outline"
              title="No models found"
              body={`Relay looked in:\n${data.dirs.join('\n')}`}
            />
          ) : undefined
        }
        renderItem={({ item, index }) => {
          const first = index === 0;
          const last = index === (data?.models.length ?? 0) - 1;
          const current = item.path === data?.current;
          const tooBig = budget > 0 && item.size_bytes > budget;
          return (
            <View style={[styles.itemOuter, first && styles.first, last && styles.last, current && styles.itemCurrent]}>
              <Pressable
                onPress={() => choose(item)}
                disabled={busy !== null}
                style={({ pressed }) => [styles.item, pressed && styles.pressed]}
                accessibilityRole="button"
                accessibilityState={{ selected: current }}
                accessibilityLabel={`${item.name}${current ? ', current model' : ''}`}
              >
                <View style={[styles.icon, current && styles.iconCurrent]}>
                  <Icon name="cube-outline" size={22} color={current ? m3.primary : m3.onSurfaceVariant} />
                </View>
                <View style={styles.grow}>
                  <Text style={[type.bodyLarge, current && { color: m3.primary }]} numberOfLines={2}>
                    {item.name}
                  </Text>
                  <View style={styles.metaRow}>
                    <Text style={type.bodyMedium}>{formatBytes(item.size_bytes)}</Text>
                    {tooBig ? (
                      <View style={styles.warnChip}>
                        <Text style={[type.labelMedium, { color: m3.onWarningContainer }]}>May not fit in VRAM</Text>
                      </View>
                    ) : null}
                  </View>
                  <TagChips tags={item.tags ?? []} muted={!item.custom_tags} />
                  <Text style={type.bodySmall} numberOfLines={1} ellipsizeMode="middle">
                    {item.path}
                  </Text>
                </View>
                {busy === item.path ? (
                  <ActivityIndicator color={m3.primary} />
                ) : current ? (
                  <Icon name="check-circle" size={24} color={m3.primary} />
                ) : null}
              </Pressable>
              <IconButton
                icon="tag-edit-outline"
                label={`Edit tags for ${item.name}`}
                onPress={() => setTagging({ name: item.name, tags: item.tags ?? [], custom: !!item.custom_tags })}
              />
            </View>
          );
        }}
      />
      <TagsSheet
        model={tagging}
        onClose={() => setTagging(null)}
        onSave={async (tags) => {
          if (!tagging) return;
          await client.setModelTags(tagging.name, tags);
          setTagging(null);
          await load();
        }}
      />
    </>
  );
}

const styles = StyleSheet.create({
  flex: { flex: 1, backgroundColor: m3.surface },
  grow: { flex: 1, minWidth: 0 },
  content: { padding: space.lg, paddingBottom: 48 },
  intro: { marginBottom: space.lg, marginHorizontal: space.xs },
  itemOuter: {
    flexDirection: 'row',
    alignItems: 'center',
    backgroundColor: m3.surfaceContainerLow,
    marginBottom: 2,
    borderRadius: radius.xs,
    overflow: 'hidden',
    paddingRight: space.xs,
  },
  item: {
    flex: 1,
    minWidth: 0,
    flexDirection: 'row',
    alignItems: 'center',
    gap: space.lg,
    minHeight: 72,
    paddingHorizontal: space.lg,
    paddingVertical: space.md,
  },
  first: { borderTopLeftRadius: radius.xl, borderTopRightRadius: radius.xl },
  last: { borderBottomLeftRadius: radius.xl, borderBottomRightRadius: radius.xl },
  itemCurrent: { backgroundColor: m3.surfaceContainerHigh },
  pressed: { backgroundColor: m3.surfaceContainerHighest },
  icon: {
    width: 40,
    height: 40,
    borderRadius: radius.md,
    backgroundColor: m3.surfaceContainerHighest,
    alignItems: 'center',
    justifyContent: 'center',
  },
  iconCurrent: { backgroundColor: m3.surfaceContainerHighest },
  metaRow: { flexDirection: 'row', alignItems: 'center', gap: space.sm, marginTop: 2 },
  warnChip: {
    backgroundColor: m3.warningContainer,
    borderRadius: radius.sm,
    paddingHorizontal: space.sm,
    paddingVertical: 2,
  },
});
