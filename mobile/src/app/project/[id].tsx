import { Stack, router, useFocusEffect, useLocalSearchParams } from 'expo-router';
import { useCallback, useEffect, useRef, useState } from 'react';
import {
  ActivityIndicator,
  Alert,
  FlatList,
  KeyboardAvoidingView,
  Modal,
  Platform,
  Pressable,
  RefreshControl,
  StyleSheet,
  Text,
  View,
} from 'react-native';
import { useSafeAreaInsets } from 'react-native-safe-area-context';

import { colors, radius, space, touch } from '../../components/theme';
import { Banner, Button, EmptyState, Field, StatusBadge } from '../../components/ui';
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
      Alert.alert('Could not create a session', describeError(e));
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
          Alert.alert('Delete this session?', 'Its messages are removed from Relay. Files on your PC are not touched.', [
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
      <Stack.Screen options={{ title: name ?? 'Sessions' }} />
      {error ? <Banner message={error} actionLabel="Retry" onAction={load} /> : null}
      <FlatList
        data={sessions ?? []}
        keyExtractor={(s) => s.id}
        contentContainerStyle={styles.list}
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
        ListEmptyComponent={
          sessions === null ? (
            error ? undefined : (
              <ActivityIndicator style={{ marginTop: space.xl }} color={colors.accent} />
            )
          ) : (
            <EmptyState title="No sessions yet" body="Start a new session and send the agent a prompt." />
          )
        }
        renderItem={({ item }) => (
          <Pressable
            onPress={() => router.push({ pathname: '/session/[id]', params: { id: item.id, title: item.title } })}
            onLongPress={() => actions(item)}
            style={({ pressed }) => [styles.row, pressed && styles.rowPressed]}
            accessibilityRole="button"
            accessibilityHint="Opens the chat. Long press to rename or delete."
          >
            <View style={styles.rowTop}>
              <Text style={styles.rowTitle} numberOfLines={1}>
                {item.title}
              </Text>
              <StatusBadge status={item.status} />
            </View>
            <View style={styles.rowBottom}>
              <Text style={styles.rowSub} numberOfLines={1}>
                {item.status === 'error' && item.error ? item.error : `Active ${relativeTime(item.updated_at)}`}
              </Text>
              <Pressable
                onPress={() => actions(item)}
                hitSlop={10}
                style={styles.more}
                accessibilityRole="button"
                accessibilityLabel={`More actions for ${item.title}`}
              >
                <Text style={styles.moreText}>•••</Text>
              </Pressable>
            </View>
          </Pressable>
        )}
      />
      <View style={[styles.bottom, { paddingBottom: Math.max(insets.bottom, space.md) }]}>
        <Button title="＋  New session" onPress={newSession} loading={creating} />
      </View>
      <RenameModal
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

function RenameModal({
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
  const insets = useSafeAreaInsets();

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
    <Modal visible={session !== null} animationType="slide" transparent onRequestClose={onClose}>
      <KeyboardAvoidingView style={styles.modalWrap} behavior={Platform.OS === 'ios' ? 'padding' : 'height'}>
        <Pressable style={styles.backdrop} onPress={onClose} accessibilityLabel="Close" />
        <View style={[styles.sheet, { paddingBottom: Math.max(insets.bottom, space.lg) }]}>
          <Text style={styles.sheetTitle}>Rename session</Text>
          <Field label="Title" value={title} onChangeText={setTitle} autoFocus autoCapitalize="sentences" onSubmitEditing={submit} />
          {error ? <Banner message={error} /> : null}
          <View style={styles.sheetActions}>
            <Button title="Cancel" variant="secondary" onPress={onClose} style={styles.flex} />
            <Button title="Save" onPress={submit} loading={busy} disabled={!title.trim()} style={styles.flex} />
          </View>
        </View>
      </KeyboardAvoidingView>
    </Modal>
  );
}

const styles = StyleSheet.create({
  flex: { flex: 1 },
  list: { padding: space.lg, paddingTop: space.sm },
  row: {
    minHeight: touch + 24,
    backgroundColor: colors.surface,
    borderRadius: radius.lg,
    borderWidth: StyleSheet.hairlineWidth,
    borderColor: colors.border,
    paddingLeft: space.lg,
    paddingTop: space.md,
    marginTop: space.sm,
  },
  rowPressed: { backgroundColor: colors.surfaceRaised },
  rowTop: { flexDirection: 'row', alignItems: 'center', gap: space.sm, paddingRight: space.lg },
  rowTitle: { flex: 1, color: colors.text, fontSize: 17, fontWeight: '600' },
  rowBottom: { flexDirection: 'row', alignItems: 'center' },
  rowSub: { flex: 1, color: colors.textMuted, fontSize: 13 },
  more: { width: touch, height: 40, alignItems: 'center', justifyContent: 'center' },
  moreText: { color: colors.textFaint, fontSize: 14, letterSpacing: 1 },
  bottom: {
    paddingHorizontal: space.lg,
    paddingTop: space.sm,
    backgroundColor: colors.bg,
    borderTopWidth: StyleSheet.hairlineWidth,
    borderTopColor: colors.border,
  },
  modalWrap: { flex: 1, justifyContent: 'flex-end' },
  backdrop: { ...StyleSheet.absoluteFill, backgroundColor: 'rgba(0,0,0,0.55)' },
  sheet: { backgroundColor: colors.surface, borderTopLeftRadius: radius.lg, borderTopRightRadius: radius.lg, padding: space.lg },
  sheetTitle: { color: colors.text, fontSize: 20, fontWeight: '700', marginBottom: space.lg },
  sheetActions: { flexDirection: 'row', gap: space.md },
});
