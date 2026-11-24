import { Stack, router, useFocusEffect, useLocalSearchParams } from 'expo-router';
import { useCallback, useEffect, useMemo, useState } from 'react';
import { ActivityIndicator, Pressable, RefreshControl, ScrollView, StyleSheet, Text, View } from 'react-native';
import { useSafeAreaInsets } from 'react-native-safe-area-context';

import { alert } from '../../components/Dialogs';
import { ImageModelChip, ImageModelSheet, StoredImage } from '../../components/ImageBits';
import { Sheet } from '../../components/Sheet';
import { m3, radius, space, type } from '../../components/theme';
import { Banner, Button, EmptyState, Fab, Field, Icon, IconButton } from '../../components/ui';
import { describeError } from '../../lib/api';
import { useClient } from '../../lib/connection';
import { emit } from '../../lib/events';
import { relativeTime } from '../../lib/format';
import { imagesApi, type ImageChat, type ImageModel, type ImageProject } from '../../lib/images';
import { usePageStyle, useWide } from '../../lib/layout';

/** An image project: its image chats as a gallery, plus the look shared by all of them. */
export default function ImageProjectScreen() {
  const client = useClient();
  const api = useMemo(() => (client ? imagesApi(client) : null), [client]);
  const { id, name } = useLocalSearchParams<{ id: string; name?: string }>();
  const insets = useSafeAreaInsets();
  const page = usePageStyle(1000);
  const wide = useWide();
  const [project, setProject] = useState<ImageProject | null>(null);
  const [chats, setChats] = useState<ImageChat[] | null>(null);
  const [models, setModels] = useState<ImageModel[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [refreshing, setRefreshing] = useState(false);
  const [creating, setCreating] = useState(false);
  const [picking, setPicking] = useState(false);
  const [editing, setEditing] = useState(false);
  const [gridWidth, setGridWidth] = useState(0);

  const load = useCallback(async () => {
    if (!api || !id) return;
    try {
      const [p, c] = await Promise.all([api.project(id), api.chats(id)]);
      setProject(p);
      setChats(c);
      setError(null);
    } catch (e) {
      setError(describeError(e));
    }
  }, [api, id]);

  useFocusEffect(
    useCallback(() => {
      load();
      const t = setInterval(load, 5000);
      return () => clearInterval(t);
    }, [load]),
  );
  useEffect(() => {
    api?.models().then(setModels).catch(() => undefined);
  }, [api]);

  if (!api) return null;

  const newChat = async () => {
    setCreating(true);
    try {
      const c = await api.createChat(id);
      emit('chats');
      router.push({ pathname: '/image/[id]', params: { id: c.id, title: c.title } });
    } catch (e) {
      alert("Couldn't start an image chat", describeError(e));
    } finally {
      setCreating(false);
    }
  };

  const chatActions = (c: ImageChat) =>
    alert(c.title, undefined, [
      {
        text: 'Delete',
        style: 'destructive',
        onPress: () =>
          alert('Delete this image chat?', 'Its pictures are removed from Relay.', [
            { text: 'Cancel', style: 'cancel' },
            {
              text: 'Delete',
              style: 'destructive',
              onPress: async () => {
                try {
                  await api.deleteChat(c.id);
                  emit('chats');
                  load();
                } catch (e) {
                  alert("Couldn't delete", describeError(e));
                }
              },
            },
          ]),
      },
      { text: 'Cancel', style: 'cancel' },
    ]);

  const model = models.find((m) => m.id === project?.model_id);
  const cols = wide ? 4 : 2;
  const GAP = space.md;
  const tileWidth = gridWidth ? (gridWidth - GAP * (cols - 1)) / cols : 0;

  return (
    <View style={styles.flex}>
      <Stack.Screen
        options={{
          title: project?.name ?? name ?? 'Images',
          headerRight: () => (
            <IconButton icon="pencil-outline" label="Project style and name" onPress={() => setEditing(true)} />
          ),
        }}
      />
      {error ? <Banner message={error} actionLabel="Retry" onAction={load} /> : null}
      <ScrollView
        contentContainerStyle={[styles.content, page, { paddingBottom: insets.bottom + 112 }]}
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
      >
        <View style={styles.infoCard}>
          <View style={styles.infoRow}>
            <Text style={[type.bodyMedium, styles.muted, styles.grow]}>Model for new image chats</Text>
            <ImageModelChip label={model?.name ?? (models.length ? 'Latest set up' : 'None yet')} onPress={() => setPicking(true)} />
          </View>
          <Pressable onPress={() => setEditing(true)} style={styles.styleRow} accessibilityRole="button">
            <Icon name="palette-swatch-outline" size={20} color={m3.onSurfaceVariant} />
            <View style={styles.grow}>
              <Text style={type.labelLarge}>Project style</Text>
              <Text style={[type.bodySmall, styles.muted]} numberOfLines={2}>
                {project?.style
                  ? project.style
                  : 'Add a look every picture here shares, like "flat vector, pastel colours".'}
              </Text>
            </View>
            <Icon name="chevron-right" size={20} color={m3.onSurfaceVariant} />
          </Pressable>
        </View>

        {chats === null ? (
          <ActivityIndicator color={m3.primary} style={{ marginTop: space.xl }} />
        ) : chats.length === 0 ? (
          <EmptyState
            icon="image-plus"
            title="Start an image chat"
            body="Describe a picture, then keep refining it message by message."
          />
        ) : (
          <View style={styles.grid} onLayout={(e) => setGridWidth(e.nativeEvent.layout.width)}>
            {chats.map((c) => (
              <Pressable
                key={c.id}
                onPress={() => router.push({ pathname: '/image/[id]', params: { id: c.id, title: c.title } })}
                onLongPress={() => chatActions(c)}
                style={({ pressed, hovered }: { pressed: boolean; hovered?: boolean }) => [
                  styles.tile,
                  { width: tileWidth || `${Math.floor(100 / cols) - 3}%` },
                  (pressed || hovered) && styles.tileHover,
                ]}
                accessibilityRole="button"
                accessibilityLabel={c.title}
                accessibilityHint="Opens the image chat. Long press to delete."
              >
                <View style={styles.thumb}>
                  {c.cover ? (
                    <StoredImage name={c.cover} style={StyleSheet.absoluteFill} />
                  ) : (
                    <Icon name="image-outline" size={32} color={m3.outline} />
                  )}
                  {c.status === 'running' ? (
                    <View style={styles.drawing}>
                      <ActivityIndicator size="small" color="#fff" />
                    </View>
                  ) : null}
                </View>
                <View style={styles.tileText}>
                  <Text style={[type.labelLarge, styles.grow]} numberOfLines={1}>
                    {c.title}
                  </Text>
                  <IconButton icon="dots-vertical" size={18} label={`More for ${c.title}`} onPress={() => chatActions(c)} />
                </View>
                <Text style={[type.bodySmall, styles.muted, styles.tileSub]}>
                  {c.status === 'running' ? 'Drawing…' : relativeTime(c.updated_at)}
                </Text>
              </Pressable>
            ))}
          </View>
        )}
      </ScrollView>
      <Fab
        icon={creating ? 'timer-sand' : 'image-plus'}
        label="New image chat"
        onPress={creating ? () => undefined : newChat}
        style={[styles.fab, { bottom: insets.bottom + space.lg }]}
      />
      <ImageModelSheet
        visible={picking}
        models={models}
        value={project?.model_id}
        title="Model for new image chats"
        onClose={() => setPicking(false)}
        onPick={async (mid) => {
          setProject(await api.updateProject(id, { model_id: mid }));
          setPicking(false);
        }}
      />
      <EditProjectSheet
        visible={editing}
        project={project}
        onClose={() => setEditing(false)}
        onSave={async (p) => {
          setProject(await api.updateProject(id, p));
          emit('projects');
          setEditing(false);
        }}
        onDelete={() =>
          alert('Delete this image project?', 'All its chats and pictures are removed from Relay.', [
            { text: 'Cancel', style: 'cancel' },
            {
              text: 'Delete',
              style: 'destructive',
              onPress: async () => {
                try {
                  await api.deleteProject(id);
                  emit('projects');
                  setEditing(false);
                  router.replace('/');
                } catch (e) {
                  alert("Couldn't delete", describeError(e));
                }
              },
            },
          ])
        }
      />
    </View>
  );
}

function EditProjectSheet({
  visible,
  project,
  onClose,
  onSave,
  onDelete,
}: {
  visible: boolean;
  project: ImageProject | null;
  onClose: () => void;
  onSave: (p: { name: string; style: string; negative: string }) => Promise<void>;
  onDelete: () => void;
}) {
  const [name, setName] = useState('');
  const [style, setStyle] = useState('');
  const [negative, setNegative] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    if (!visible || !project) return;
    setName(project.name);
    setStyle(project.style);
    setNegative(project.negative);
    setError(null);
  }, [visible, project]);
  return (
    <Sheet visible={visible} onClose={onClose} title="Image project">
      <Field label="Name" value={name} onChangeText={setName} autoCapitalize="sentences" />
      <Field
        label="Style"
        value={style}
        onChangeText={setStyle}
        multiline
        placeholder="flat vector illustration, pastel colours"
        hint="Added to every prompt in this project"
      />
      <Field
        label="Leave out"
        value={negative}
        onChangeText={setNegative}
        placeholder="text, watermark, blurry"
        hint="Kept out of every picture in this project"
      />
      {error ? <Banner message={error} /> : null}
      <View style={styles.sheetActions}>
        <Button title="Delete project" variant="text" onPress={onDelete} />
        <View style={styles.grow} />
        <Button title="Cancel" variant="text" onPress={onClose} />
        <Button
          title="Save"
          loading={busy}
          disabled={!name.trim()}
          onPress={async () => {
            setBusy(true);
            try {
              await onSave({ name: name.trim(), style: style.trim(), negative: negative.trim() });
            } catch (e) {
              setError(describeError(e));
            } finally {
              setBusy(false);
            }
          }}
        />
      </View>
    </Sheet>
  );
}

const styles = StyleSheet.create({
  flex: { flex: 1, backgroundColor: m3.surface },
  grow: { flex: 1, minWidth: 0 },
  muted: { color: m3.onSurfaceVariant },
  content: { paddingHorizontal: space.lg, paddingTop: space.sm, gap: space.lg },
  infoCard: { backgroundColor: m3.surfaceContainerLow, borderRadius: radius.xl, padding: space.md, gap: space.xs },
  infoRow: { flexDirection: 'row', alignItems: 'center', gap: space.md, paddingLeft: space.sm },
  styleRow: {
    flexDirection: 'row',
    alignItems: 'center',
    gap: space.md,
    padding: space.sm,
    borderRadius: radius.md,
  },
  grid: { flexDirection: 'row', flexWrap: 'wrap', columnGap: space.md, rowGap: space.lg },
  tile: { borderRadius: radius.lg, padding: space.xs },
  tileHover: { backgroundColor: m3.hover },
  thumb: {
    width: '100%',
    aspectRatio: 1,
    borderRadius: radius.lg,
    overflow: 'hidden',
    backgroundColor: m3.surfaceContainerHigh,
    alignItems: 'center',
    justifyContent: 'center',
  },
  drawing: {
    position: 'absolute',
    right: space.sm,
    bottom: space.sm,
    width: 32,
    height: 32,
    borderRadius: 16,
    backgroundColor: 'rgba(0,0,0,0.55)',
    alignItems: 'center',
    justifyContent: 'center',
  },
  tileText: { flexDirection: 'row', alignItems: 'center', paddingLeft: space.xs, marginTop: space.xs },
  tileSub: { paddingLeft: space.xs, marginTop: -6 },
  fab: { position: 'absolute', right: space.lg },
  sheetActions: { flexDirection: 'row', alignItems: 'center', gap: space.sm, marginTop: space.sm },
});
