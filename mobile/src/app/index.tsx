import { Redirect, Stack, router, useFocusEffect } from 'expo-router';
import { useCallback, useState } from 'react';
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

import { ModelStatusBar } from '../components/ModelStatusBar';
import { colors, radius, space, touch } from '../components/theme';
import { Banner, Button, EmptyState, Field } from '../components/ui';
import { describeError } from '../lib/api';
import { useConnection } from '../lib/connection';
import type { Project } from '../lib/types';

export default function ProjectsScreen() {
  const { loading, client } = useConnection();
  const insets = useSafeAreaInsets();
  const [projects, setProjects] = useState<Project[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [refreshing, setRefreshing] = useState(false);
  const [focused, setFocused] = useState(false);
  const [adding, setAdding] = useState(false);

  const load = useCallback(async () => {
    if (!client) return;
    try {
      setProjects(await client.projects());
      setError(null);
    } catch (e) {
      setError(describeError(e));
    }
  }, [client]);

  useFocusEffect(
    useCallback(() => {
      setFocused(true);
      load();
      return () => setFocused(false);
    }, [load]),
  );

  if (loading) {
    return (
      <View style={styles.center}>
        <ActivityIndicator color={colors.accent} />
      </View>
    );
  }
  if (!client) return <Redirect href="/connect" />;

  const onRefresh = async () => {
    setRefreshing(true);
    await load();
    setRefreshing(false);
  };

  const confirmDelete = (p: Project) => {
    Alert.alert(
      `Remove “${p.name}”?`,
      'This removes the project and its sessions from Relay. The folder on your PC is not touched.',
      [
        { text: 'Cancel', style: 'cancel' },
        {
          text: 'Remove',
          style: 'destructive',
          onPress: async () => {
            try {
              await client.deleteProject(p.id);
              setProjects((list) => list?.filter((x) => x.id !== p.id) ?? null);
            } catch (e) {
              Alert.alert('Could not remove project', describeError(e));
            }
          },
        },
      ],
    );
  };

  return (
    <View style={styles.flex}>
      <Stack.Screen
        options={{
          headerRight: () => (
            <Pressable
              onPress={() => router.push('/settings')}
              hitSlop={12}
              style={styles.headerBtn}
              accessibilityRole="button"
              accessibilityLabel="Settings"
            >
              <Text style={styles.headerBtnText}>⚙︎</Text>
            </Pressable>
          ),
        }}
      />
      <ModelStatusBar client={client} active={focused} onPress={() => router.push('/settings')} />
      {error ? <Banner message={error} actionLabel="Retry" onAction={load} /> : null}
      <FlatList
        data={projects ?? []}
        keyExtractor={(p) => p.id}
        contentContainerStyle={styles.list}
        refreshControl={<RefreshControl refreshing={refreshing} onRefresh={onRefresh} tintColor={colors.textMuted} />}
        ListEmptyComponent={
          projects === null ? (
            error ? undefined : (
              <ActivityIndicator style={{ marginTop: space.xl }} color={colors.accent} />
            )
          ) : (
            <EmptyState title="No projects yet" body="Add a folder on your PC to start a coding session in it." />
          )
        }
        renderItem={({ item }) => (
          <Pressable
            onPress={() => router.push({ pathname: '/project/[id]', params: { id: item.id, name: item.name } })}
            onLongPress={() => confirmDelete(item)}
            style={({ pressed }) => [styles.row, pressed && styles.rowPressed]}
            accessibilityRole="button"
            accessibilityHint="Opens sessions. Long press to remove."
          >
            <View style={styles.rowText}>
              <Text style={styles.rowTitle} numberOfLines={1}>
                {item.name}
              </Text>
              <Text style={styles.rowSub} numberOfLines={1} ellipsizeMode="middle">
                {item.path}
              </Text>
            </View>
            <Pressable
              onPress={() => confirmDelete(item)}
              hitSlop={10}
              style={styles.trash}
              accessibilityRole="button"
              accessibilityLabel={`Remove ${item.name}`}
            >
              <Text style={styles.trashText}>✕</Text>
            </Pressable>
          </Pressable>
        )}
      />
      <View style={[styles.bottom, { paddingBottom: Math.max(insets.bottom, space.md) }]}>
        <Button title="＋  Add project" onPress={() => setAdding(true)} />
      </View>
      <AddProjectModal
        visible={adding}
        onClose={() => setAdding(false)}
        onCreate={async (name, path) => {
          const p = await client.createProject(name, path);
          setProjects((list) => [...(list ?? []), p].sort((a, b) => a.name.localeCompare(b.name)));
          setAdding(false);
        }}
      />
    </View>
  );
}

function AddProjectModal({
  visible,
  onClose,
  onCreate,
}: {
  visible: boolean;
  onClose: () => void;
  onCreate: (name: string, path: string) => Promise<void>;
}) {
  const [name, setName] = useState('');
  const [path, setPath] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const insets = useSafeAreaInsets();

  const submit = async () => {
    setBusy(true);
    setError(null);
    try {
      await onCreate(name.trim(), path.trim());
      setName('');
      setPath('');
    } catch (e) {
      setError(describeError(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Modal visible={visible} animationType="slide" transparent onRequestClose={onClose}>
      <KeyboardAvoidingView style={styles.modalWrap} behavior={Platform.OS === 'ios' ? 'padding' : 'height'}>
        <Pressable style={styles.backdrop} onPress={onClose} accessibilityLabel="Close" />
        <View style={[styles.sheet, { paddingBottom: Math.max(insets.bottom, space.lg) }]}>
          <Text style={styles.sheetTitle}>Add project</Text>
          <Field
            label="Folder on your PC"
            placeholder="C:\Users\me\code\my-app"
            value={path}
            onChangeText={setPath}
            hint="Absolute path. Relay only works inside this folder."
            autoFocus
          />
          <Field label="Name (optional)" placeholder="Defaults to the folder name" value={name} onChangeText={setName} />
          {error ? <Banner message={error} /> : null}
          <View style={styles.sheetActions}>
            <Button title="Cancel" variant="secondary" onPress={onClose} style={styles.flex} />
            <Button title="Add" onPress={submit} loading={busy} disabled={!path.trim()} style={styles.flex} />
          </View>
        </View>
      </KeyboardAvoidingView>
    </Modal>
  );
}

const styles = StyleSheet.create({
  flex: { flex: 1 },
  center: { flex: 1, alignItems: 'center', justifyContent: 'center', backgroundColor: colors.bg },
  list: { paddingHorizontal: space.lg, paddingBottom: space.lg },
  row: {
    minHeight: touch + 20,
    flexDirection: 'row',
    alignItems: 'center',
    backgroundColor: colors.surface,
    borderRadius: radius.lg,
    borderWidth: StyleSheet.hairlineWidth,
    borderColor: colors.border,
    paddingLeft: space.lg,
    marginTop: space.sm,
  },
  rowPressed: { backgroundColor: colors.surfaceRaised },
  rowText: { flex: 1, paddingVertical: space.md },
  rowTitle: { color: colors.text, fontSize: 17, fontWeight: '600' },
  rowSub: { color: colors.textMuted, fontSize: 13, marginTop: 2 },
  trash: { width: touch, height: touch + 20, alignItems: 'center', justifyContent: 'center' },
  trashText: { color: colors.textFaint, fontSize: 16 },
  bottom: {
    paddingHorizontal: space.lg,
    paddingTop: space.sm,
    backgroundColor: colors.bg,
    borderTopWidth: StyleSheet.hairlineWidth,
    borderTopColor: colors.border,
  },
  headerBtn: { minWidth: 40, height: 40, alignItems: 'center', justifyContent: 'center' },
  headerBtnText: { color: colors.text, fontSize: 22 },
  modalWrap: { flex: 1, justifyContent: 'flex-end' },
  backdrop: { ...StyleSheet.absoluteFill, backgroundColor: 'rgba(0,0,0,0.55)' },
  sheet: {
    backgroundColor: colors.surface,
    borderTopLeftRadius: radius.lg,
    borderTopRightRadius: radius.lg,
    padding: space.lg,
  },
  sheetTitle: { color: colors.text, fontSize: 20, fontWeight: '700', marginBottom: space.lg },
  sheetActions: { flexDirection: 'row', gap: space.md },
});
