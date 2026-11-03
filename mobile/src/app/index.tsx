import { Redirect, Stack, router, useFocusEffect } from 'expo-router';
import { useCallback, useState } from 'react';
import { ActivityIndicator, FlatList, RefreshControl, StyleSheet, Text, View } from 'react-native';
import { useSafeAreaInsets } from 'react-native-safe-area-context';

import { DesktopHome } from '../components/DesktopHome';
import { ModelStatusBar } from '../components/ModelStatusBar';
import { m3, radius, space, type } from '../components/theme';
import { AddProjectSheet } from '../components/AddProjectSheet';
import { Avatar, Banner, Logo, Button, EmptyState, Fab, Field, IconButton, ListItem } from '../components/ui';
import { emit } from '../lib/events';
import { alert } from '../components/Dialogs';
import { describeError } from '../lib/api';
import { useConnection } from '../lib/connection';
import { useWide } from '../lib/layout';
import type { Project } from '../lib/types';

function greeting(): string {
  const h = new Date().getHours();
  if (h < 5) return 'Up late?';
  if (h < 12) return 'Good morning';
  if (h < 18) return 'Good afternoon';
  return 'Good evening';
}

/** Phones get the projects list; the desktop app gets a Claude-style start page. */
export default function Home() {
  const { loading, client } = useConnection();
  const wide = useWide();
  if (!loading && client && wide) return <DesktopHome />;
  return <ProjectsScreen />;
}

function ProjectsScreen() {
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
        <ActivityIndicator color={m3.primary} />
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
    alert(
      `Remove “${p.name}”?`,
      'This removes the project and its chats from Relay. The folder on your PC is not touched.',
      [
        { text: 'Cancel', style: 'cancel' },
        {
          text: 'Remove',
          style: 'destructive',
          onPress: async () => {
            try {
              await client.deleteProject(p.id);
              emit('projects');
              setProjects((list) => list?.filter((x) => x.id !== p.id) ?? null);
            } catch (e) {
              alert('Could not remove project', describeError(e));
            }
          },
        },
      ],
    );
  };

  const header = (
    <View>
      <Text style={[type.displaySmall, styles.greeting]}>{greeting()}</Text>
      <Text style={[type.bodyLarge, styles.sub]}>Pick a project to chat with your agent.</Text>
      <ModelStatusBar client={client} active={focused} onPress={() => router.push('/settings')} />
      {error ? <Banner message={error} actionLabel="Retry" onAction={load} /> : null}
      {projects && projects.length > 0 ? <Text style={[type.titleSmall, styles.section]}>Projects</Text> : null}
    </View>
  );

  return (
    <View style={[styles.flex, { paddingTop: insets.top }]}>
      <Stack.Screen options={{ headerShown: false }} />
      <View style={styles.appBar}>
        <View style={styles.brand}>
          <Logo size={32} />
          <Text style={type.titleLarge}>Relay</Text>
        </View>
        <IconButton icon="cog-outline" label="Settings" onPress={() => router.push('/settings')} />
      </View>
      <FlatList
        data={projects ?? []}
        keyExtractor={(p) => p.id}
        ListHeaderComponent={header}
        contentContainerStyle={[styles.list, { paddingBottom: insets.bottom + 112 }]}
        refreshControl={
          <RefreshControl refreshing={refreshing} onRefresh={onRefresh} tintColor={m3.onSurfaceVariant} />
        }
        ListEmptyComponent={
          projects === null ? (
            error ? undefined : (
              <ActivityIndicator style={{ marginTop: space.xl }} color={m3.primary} />
            )
          ) : (
            <EmptyState
              icon="folder-plus-outline"
              title="No projects yet"
              body="Give it a name and Relay makes a folder for it on your PC. The agent works inside it."
            />
          )
        }
        renderItem={({ item, index }) => {
          const first = index === 0;
          const last = index === (projects?.length ?? 0) - 1;
          return (
            <View style={[styles.itemWrap, first && styles.itemFirst, last && styles.itemLast]}>
              <ListItem
                title={item.name}
                subtitle={item.path}
                leading={<Avatar label={item.name} />}
                trailing={
                  <IconButton icon="dots-vertical" label={`Remove ${item.name}`} onPress={() => confirmDelete(item)} />
                }
                onPress={() => router.push({ pathname: '/project/[id]', params: { id: item.id, name: item.name } })}
                onLongPress={() => confirmDelete(item)}
                accessibilityHint="Opens chats. Long press to remove."
              />
              {!last ? <View style={styles.divider} /> : null}
            </View>
          );
        }}
      />
      <Fab
        icon="plus"
        label="New project"
        onPress={() => setAdding(true)}
        style={[styles.fab, { bottom: insets.bottom + space.lg }]}
      />
      <AddProjectSheet
        visible={adding}
        onClose={() => setAdding(false)}
        onCreate={async (name, path) => {
          const p = await client.createProject(name, path);
          emit('projects');
          setProjects((list) => [...(list ?? []), p].sort((a, b) => a.name.localeCompare(b.name)));
          setAdding(false);
        }}
      />
    </View>
  );
}

const styles = StyleSheet.create({
  flex: { flex: 1, backgroundColor: m3.surface },
  center: { flex: 1, alignItems: 'center', justifyContent: 'center', backgroundColor: m3.surface },
  appBar: {
    height: 64,
    flexDirection: 'row',
    alignItems: 'center',
    justifyContent: 'space-between',
    paddingLeft: space.lg,
    paddingRight: space.xs,
  },
  brand: { flexDirection: 'row', alignItems: 'center', gap: space.md },
  greeting: { marginTop: space.md, marginHorizontal: space.xs },
  sub: { color: m3.onSurfaceVariant, marginTop: space.xs, marginBottom: space.lg, marginHorizontal: space.xs },
  section: { color: m3.onSurfaceVariant, marginTop: space.xl, marginBottom: space.sm, marginLeft: space.xs },
  list: { paddingHorizontal: space.lg },
  itemWrap: { backgroundColor: m3.surfaceContainerLow, overflow: 'hidden' },
  itemFirst: { borderTopLeftRadius: radius.xl, borderTopRightRadius: radius.xl, paddingTop: space.xs },
  itemLast: { borderBottomLeftRadius: radius.xl, borderBottomRightRadius: radius.xl, paddingBottom: space.xs },
  divider: { height: StyleSheet.hairlineWidth, backgroundColor: m3.outlineVariant, marginLeft: 72 },
  fab: { position: 'absolute', right: space.lg },
});
