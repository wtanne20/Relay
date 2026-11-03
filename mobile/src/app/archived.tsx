import { router, useFocusEffect } from 'expo-router';
import { useCallback, useState } from 'react';
import { ActivityIndicator, Pressable, ScrollView, StyleSheet, Text, View } from 'react-native';

import { alert } from '../components/Dialogs';
import { m3, radius, space, type } from '../components/theme';
import { Avatar, Banner, Icon, IconButton } from '../components/ui';
import { describeError } from '../lib/api';
import { useClient } from '../lib/connection';
import { emit } from '../lib/events';
import { relativeTime } from '../lib/format';
import { usePageStyle } from '../lib/layout';
import { CHATS_ID, type Project, type RecentSession } from '../lib/types';

/** Archived chats and projects: open, restore or delete them. */
export default function ArchivedScreen() {
  const page = usePageStyle();
  const client = useClient();
  const [data, setData] = useState<{ projects: Project[]; sessions: RecentSession[] } | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);

  const load = useCallback(async () => {
    if (!client) return;
    try {
      setData(await client.archived());
      setError(null);
    } catch (e) {
      setError(describeError(e));
    }
  }, [client]);

  useFocusEffect(
    useCallback(() => {
      load();
    }, [load]),
  );

  if (!client) return null;

  const run = async (key: string, act: () => Promise<unknown>) => {
    setBusy(key);
    try {
      await act();
      emit('chats');
      emit('projects');
      await load();
    } catch (e) {
      alert('Something went wrong', describeError(e));
    } finally {
      setBusy(null);
    }
  };

  const deleteChat = (s: RecentSession) =>
    alert(`Delete “${s.title}”?`, 'This chat is deleted for good.', [
      { text: 'Cancel', style: 'cancel' },
      { text: 'Delete', style: 'destructive', onPress: () => run(`c:${s.id}`, () => client.deleteSession(s.id)) },
    ]);
  const deleteProject = (p: Project) =>
    alert(
      `Delete “${p.name}”?`,
      'The project and its chats are removed from Relay. The folder and files on your PC are kept.',
      [
        { text: 'Cancel', style: 'cancel' },
        { text: 'Delete', style: 'destructive', onPress: () => run(`p:${p.id}`, () => client.deleteProject(p.id)) },
      ],
    );

  const actions = (key: string, restore: () => Promise<unknown>, remove: () => void, label: string) =>
    busy === key ? (
      <ActivityIndicator color={m3.primary} style={styles.spinner} />
    ) : (
      <>
        <IconButton icon="archive-arrow-up-outline" label={`Restore ${label}`} onPress={() => run(key, restore)} />
        <IconButton icon="delete-outline" label={`Delete ${label}`} onPress={remove} />
      </>
    );

  const empty = data && !data.projects.length && !data.sessions.length;

  return (
    <ScrollView contentContainerStyle={[styles.content, page]}>
      {error ? <Banner message={error} actionLabel="Retry" onAction={load} /> : null}
      {!data && !error ? <ActivityIndicator color={m3.primary} style={{ margin: space.xl }} /> : null}
      {empty ? (
        <View style={styles.empty}>
          <Icon name="archive-outline" size={40} color={m3.outline} />
          <Text style={[type.bodyLarge, styles.muted]}>Nothing archived</Text>
          <Text style={[type.bodyMedium, styles.muted, { textAlign: 'center' }]}>
            On the home screen, press and hold a chat or project to select it, then tap Archive.
          </Text>
        </View>
      ) : null}

      {data?.sessions.length ? (
        <>
          <Text style={[type.titleSmall, styles.section]}>Chats</Text>
          <View style={styles.group}>
            {data.sessions.map((s, i) => (
              <View key={s.id}>
                {i > 0 ? <View style={styles.divider} /> : null}
                <View style={styles.row}>
                  <Pressable
                    onPress={() => router.push({ pathname: '/session/[id]', params: { id: s.id, title: s.title } })}
                    style={({ hovered, pressed }: { pressed: boolean; hovered?: boolean }) => [
                      styles.item,
                      (hovered || pressed) && styles.hover,
                    ]}
                    accessibilityRole="button"
                    accessibilityLabel={s.title}
                  >
                    <Icon name="chat-outline" size={20} color={m3.onSurfaceVariant} />
                    <View style={styles.grow}>
                      <Text style={type.bodyLarge} numberOfLines={1}>
                        {s.title}
                      </Text>
                      <Text style={type.bodySmall} numberOfLines={1}>
                        {s.project_id === CHATS_ID ? '' : `${s.project_name} · `}
                        {relativeTime(s.updated_at)}
                      </Text>
                    </View>
                  </Pressable>
                  {actions(
                    `c:${s.id}`,
                    () => client.archiveSession(s.id, false),
                    () => deleteChat(s),
                    s.title,
                  )}
                </View>
              </View>
            ))}
          </View>
        </>
      ) : null}

      {data?.projects.length ? (
        <>
          <Text style={[type.titleSmall, styles.section]}>Projects</Text>
          <View style={styles.group}>
            {data.projects.map((p, i) => (
              <View key={p.id}>
                {i > 0 ? <View style={styles.divider} /> : null}
                <View style={styles.row}>
                  <Pressable
                    onPress={() => router.push({ pathname: '/project/[id]', params: { id: p.id, name: p.name } })}
                    style={({ hovered, pressed }: { pressed: boolean; hovered?: boolean }) => [
                      styles.item,
                      (hovered || pressed) && styles.hover,
                    ]}
                    accessibilityRole="button"
                    accessibilityLabel={p.name}
                  >
                    <Avatar label={p.name} size={32} />
                    <Text style={[type.bodyLarge, styles.grow]} numberOfLines={1}>
                      {p.name}
                    </Text>
                  </Pressable>
                  {actions(
                    `p:${p.id}`,
                    () => client.archiveProject(p.id, false),
                    () => deleteProject(p),
                    p.name,
                  )}
                </View>
              </View>
            ))}
          </View>
        </>
      ) : null}
    </ScrollView>
  );
}

const styles = StyleSheet.create({
  content: { padding: space.lg, gap: space.md },
  section: { color: m3.onSurfaceVariant, marginTop: space.sm, marginLeft: space.xs },
  group: { backgroundColor: m3.surfaceContainerLow, borderRadius: radius.lg, overflow: 'hidden' },
  divider: { height: 1, backgroundColor: m3.outlineVariant, marginLeft: 56 },
  row: { flexDirection: 'row', alignItems: 'center', paddingRight: space.xs },
  item: {
    flex: 1,
    minWidth: 0,
    flexDirection: 'row',
    alignItems: 'center',
    gap: space.md,
    minHeight: 60,
    paddingHorizontal: space.lg,
    paddingVertical: space.sm,
  },
  grow: { flex: 1, minWidth: 0 },
  hover: { backgroundColor: m3.hover },
  spinner: { width: 96 },
  empty: { alignItems: 'center', gap: space.sm, padding: space.xxl },
  muted: { color: m3.onSurfaceVariant },
});
