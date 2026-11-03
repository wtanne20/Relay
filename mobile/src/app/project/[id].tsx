import { Stack, router, useFocusEffect, useLocalSearchParams } from 'expo-router';
import { useCallback, useEffect, useRef, useState } from 'react';
import { ActivityIndicator, Alert, FlatList, Pressable, RefreshControl, StyleSheet, Text, View } from 'react-native';
import { useSafeAreaInsets } from 'react-native-safe-area-context';

import { Sheet } from '../../components/Sheet';
import { m3, radius, space, type } from '../../components/theme';
import { Banner, Button, EmptyState, Fab, Field, IconButton, StatusBadge } from '../../components/ui';
import { describeError } from '../../lib/api';
import { useClient } from '../../lib/connection';
import { relativeTime } from '../../lib/format';
import type { Session } from '../../lib/types';

const POLL_MS = 5000;

export default function SessionsScreen() {
  const client = useClient();
  const { id, name } = useLocalSearchParams<{ id: string; name?: string }>();
  const insets = useSafeAreaInsets();
  const [sessions, setSessions] = useState<Session[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [refreshing, setRefreshing] = useState(false);
  const [creating, setCreating] = useState(false);
  const [renaming, setRenaming] = useState<Session | null>(null);
  const [, setTick] = useState(0);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);

  const load = useCallback(async () => {
    if (!client || !id) return;
    try {
      setSessions(await client.sessions(id));
      setError(null);
    } catch (e) {
      setError(describeError(e));
    }
  }, [client, id]);

  // Poll while focused so status badges stay current.
  useFocusEffect(
    useCallback(() => {
      let alive = true;
      const loop = async () => {
        await load();
        setTick((t) => t + 1);
        if (alive) timer.current = setTimeout(loop, POLL_MS);
      };
      loop();
      return () => {
        alive = false;
        if (timer.current) clearTimeout(timer.current);
      };
    }, [load]),
  );

  useEffect(() => {
    if (!client) router.replace('/connect');
  }, [client]);

  if (!client) return null;

  const newSession = async () => {
    setCreating(true);
    try {
      const s = await client.createSession(id);
      router.push({ pathname: '/session/[id]', params: { id: s.id, title: s.title } });
    } catch (e) {
      Alert.alert('Could not start a chat', describeError(e));
    } finally {
      setCreating(false);
    }
  };

  const actions = (s: Session) => {
    Alert.alert(s.title, undefined, [
      { text: 'Rename', onPress: () => setRenaming(s) },
      {
        text: 'Delete',
        style: 'destructive',
        onPress: () =>
          Alert.alert('Delete this chat?', 'Its messages are removed from Relay. Files on your PC are not touched.', [
            { text: 'Cancel', style: 'cancel' },
            {
              text: 'Delete',
              style: 'destructive',
              onPress: async () => {
                try {
                  await client.deleteSession(s.id);
                  setSessions((list) => list?.filter((x) => x.id !== s.id) ?? null);
                } catch (e) {
                  Alert.alert('Could not delete', describeError(e));
                }
              },
            },
          ]),
      },
      { text: 'Cancel', style: 'cancel' },
    ]);
  };

  return (
    <View style={styles.flex}>
      <Stack.Screen
        options={{
          title: name ?? 'Chats',
          headerRight: () => (
            <IconButton
              icon="text-box-edit-outline"
              label="Project instructions"
              onPress={() => router.push({ pathname: '/project/instructions', params: { id } })}
            />
          ),
        }}
      />
      {error ? <Banner message={error} actionLabel="Retry" onAction={load} /> : null}
      <FlatList
        data={sessions ?? []}
        keyExtractor={(s) => s.id}
        contentContainerStyle={[styles.list, { paddingBottom: insets.bottom + 112 }]}
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
          sessions && sessions.length > 0 ? (
            <Text style={[type.titleSmall, styles.section]}>Recent chats</Text>
          ) : undefined
        }
        ListEmptyComponent={
          sessions === null ? (
            error ? undefined : (
              <ActivityIndicator style={{ marginTop: space.xl }} color={m3.primary} />
            )
          ) : (
            <EmptyState
              icon="chat-plus-outline"
              title="Start a chat"
              body="Ask the agent to fix a bug, add a feature or explain the code in this project."
            />
          )
        }
        renderItem={({ item, index }) => {
          const first = index === 0;
          const last = index === (sessions?.length ?? 0) - 1;
          const sub = item.status === 'error' && item.error ? item.error : relativeTime(item.updated_at);
          return (
            <View style={[styles.itemWrap, first && styles.itemFirst, last && styles.itemLast]}>
              <Pressable
                onPress={() => router.push({ pathname: '/session/[id]', params: { id: item.id, title: item.title } })}
                onLongPress={() => actions(item)}
                style={({ pressed }) => [styles.row, pressed && styles.rowPressed]}
                accessibilityRole="button"
                accessibilityLabel={item.title}
                accessibilityHint="Opens the chat. Long press to rename or delete."
              >
                <View style={styles.grow}>
                  <Text style={type.bodyLarge} numberOfLines={1}>
                    {item.title}
                  </Text>
                  <Text style={type.bodyMedium} numberOfLines={1}>
                    {sub}
                  </Text>
                </View>
                {item.status !== 'idle' ? <StatusBadge status={item.status} /> : null}
                <IconButton
                  icon="dots-vertical"
                  label={`More actions for ${item.title}`}
                  onPress={() => actions(item)}
                />
              </Pressable>
              {!last ? <View style={styles.divider} /> : null}
            </View>
          );
        }}
      />
      <Fab
        icon={creating ? 'timer-sand' : 'pencil-outline'}
        label="New chat"
        onPress={creating ? () => undefined : newSession}
        style={[styles.fab, { bottom: insets.bottom + space.lg }]}
      />
      <RenameSheet
        session={renaming}
        onClose={() => setRenaming(null)}
        onSave={async (title) => {
          if (!renaming) return;
          const s = await client.renameSession(renaming.id, title);
          setSessions((list) => list?.map((x) => (x.id === s.id ? s : x)) ?? null);
          setRenaming(null);
        }}
      />
    </View>
  );
}

function RenameSheet({
  session,
  onClose,
  onSave,
}: {
  session: Session | null;
  onClose: () => void;
  onSave: (title: string) => Promise<void>;
}) {
  const [title, setTitle] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    setTitle(session?.title ?? '');
    setError(null);
  }, [session]);

  const submit = async () => {
    setBusy(true);
    try {
      await onSave(title.trim());
    } catch (e) {
      setError(describeError(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Sheet visible={session !== null} onClose={onClose} title="Rename chat">
      <Field
        label="Title"
        value={title}
        onChangeText={setTitle}
        autoFocus
        autoCapitalize="sentences"
        onSubmitEditing={submit}
      />
      {error ? <Banner message={error} /> : null}
      <View style={styles.sheetActions}>
        <Button title="Cancel" variant="text" onPress={onClose} />
        <Button title="Save" onPress={submit} loading={busy} disabled={!title.trim()} />
      </View>
    </Sheet>
  );
}

const styles = StyleSheet.create({
  flex: { flex: 1, backgroundColor: m3.surface },
  grow: { flex: 1, minWidth: 0 },
  list: { paddingHorizontal: space.lg, paddingTop: space.sm },
  section: { color: m3.onSurfaceVariant, marginBottom: space.sm, marginLeft: space.xs, marginTop: space.sm },
  itemWrap: { backgroundColor: m3.surfaceContainerLow, overflow: 'hidden' },
  itemFirst: { borderTopLeftRadius: radius.xl, borderTopRightRadius: radius.xl, paddingTop: space.xs },
  itemLast: { borderBottomLeftRadius: radius.xl, borderBottomRightRadius: radius.xl, paddingBottom: space.xs },
  row: {
    minHeight: 72,
    flexDirection: 'row',
    alignItems: 'center',
    gap: space.sm,
    paddingLeft: space.xl,
    paddingRight: space.xs,
  },
  rowPressed: { backgroundColor: m3.pressed },
  divider: { height: StyleSheet.hairlineWidth, backgroundColor: m3.outlineVariant, marginLeft: space.xl },
  fab: { position: 'absolute', right: space.lg },
  sheetActions: { flexDirection: 'row', justifyContent: 'flex-end', gap: space.sm, marginTop: space.sm },
});
