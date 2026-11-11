import { router, usePathname } from 'expo-router';
import { useCallback, useEffect, useRef, useState } from 'react';
import { ActivityIndicator, Pressable, ScrollView, StyleSheet, Text, View } from 'react-native';

import { useConnection } from '../lib/connection';
import { emit, on } from '../lib/events';
import { CHATS_ID, type ModelStatus, type Project, type Session } from '../lib/types';
import { AddProjectSheet } from './AddProjectSheet';
import { BrainBanner, locationLabel, useHere } from './Location';
import { m3, radius, space, statusColor, type } from './theme';
import { Icon, IconButton, Logo } from './ui';

const POLL_MS = 4000;
const CHATS_SHOWN = 8;

/**
 * The desktop sidebar (Claude-app style): new chat, projects with their
 * chats, and the model status and settings at the bottom. Polls the local
 * server so chat titles and running states stay current.
 */
export function Sidebar({ onCollapse }: { onCollapse: () => void }) {
  const { client } = useConnection();
  const pathname = usePathname();
  const [projects, setProjects] = useState<Project[] | null>(null);
  const [chats, setChats] = useState<Record<string, Session[]>>({});
  const [plain, setPlain] = useState<Session[]>([]);
  const [open, setOpen] = useState<Record<string, boolean>>({});
  const [model, setModel] = useState<ModelStatus | null>(null);
  const [adding, setAdding] = useState(false);
  const openRef = useRef(open);
  openRef.current = open;

  const activeSession = pathname.startsWith('/session/') ? pathname.slice('/session/'.length) : null;
  const activeProject = pathname.startsWith('/project/') ? pathname.split('/')[2] : null;

  const load = useCallback(async () => {
    if (!client) return;
    try {
      const [ps, m, pl] = await Promise.all([
        client.projects(),
        client.model(),
        client.sessions(CHATS_ID).catch(() => [] as Session[]),
      ]);
      setProjects(ps);
      setModel(m);
      setPlain(pl);
      // Expand the first project by default so a fresh install shows something.
      const expanded = Object.keys(openRef.current).length ? openRef.current : ps[0] ? { [ps[0].id]: true } : {};
      if (expanded !== openRef.current) setOpen(expanded);
      const ids = ps.filter((p) => expanded[p.id]).map((p) => p.id);
      const lists = await Promise.all(ids.map((id) => client.sessions(id).catch(() => [] as Session[])));
      setChats((prev) => {
        const next = { ...prev };
        ids.forEach((id, i) => (next[id] = lists[i]));
        return next;
      });
    } catch {
      // The status row at the bottom shows when the server is unreachable.
      setModel(null);
    }
  }, [client]);

  // Reload on navigation (a chat was created or renamed) and every few seconds.
  useEffect(() => {
    load();
  }, [load, pathname, open]);
  useEffect(() => {
    const t = setInterval(load, POLL_MS);
    const offProjects = on('projects', load);
    const offChats = on('chats', load);
    return () => {
      clearInterval(t);
      offProjects();
      offChats();
    };
  }, [load]);

  // When a chat is open, make sure its project is expanded.
  useEffect(() => {
    if (!activeSession || !client) return;
    client
      .session(activeSession)
      .then((s) => setOpen((o) => (o[s.project_id] ? o : { ...o, [s.project_id]: true })))
      .catch(() => undefined);
  }, [activeSession, client]);

  const go = (href: Parameters<typeof router.replace>[0]) => router.replace(href);
  const here = useHere();

  return (
    <View style={styles.side}>
      <View style={styles.brand}>
        <Logo size={28} />
        <Text style={[type.titleMedium, styles.grow]}>Relay</Text>
        <IconButton icon="dock-left" label="Hide sidebar" onPress={onCollapse} size={20} />
      </View>

      <Pressable
        onPress={() => go('/')}
        style={({ pressed, hovered }: { pressed: boolean; hovered?: boolean }) => [
          styles.newChat,
          (hovered || pressed) && styles.newChatHover,
        ]}
        accessibilityRole="button"
        accessibilityLabel="New chat"
      >
        <Icon name="plus" size={20} color={m3.primary} />
        <Text style={type.labelLarge}>New chat</Text>
        <Text style={[type.labelMedium, styles.kbd]}>Ctrl N</Text>
      </Pressable>

      <View style={styles.brainRow}>
        <BrainBanner onPress={() => go('/devices')} />
      </View>

      <View style={styles.homeRow}>
        <Row icon="home-outline" label="Home" active={pathname === '/'} onPress={() => go('/')} />
      </View>

      <ScrollView style={styles.grow} contentContainerStyle={styles.list}>
        <View style={styles.sectionRow}>
          <Text style={[type.labelMedium, styles.section]}>CHATS</Text>
        </View>
        {plain.length === 0 ? <Text style={[type.bodySmall, styles.noChats]}>No chats yet</Text> : null}
        {plain.slice(0, CHATS_SHOWN).map((s) => (
          <Row
            key={s.id}
            label={s.title}
            small
            active={activeSession === s.id}
            status={s.status}
            onPress={() => go({ pathname: '/session/[id]', params: { id: s.id, title: s.title } })}
          />
        ))}
        {plain.length > CHATS_SHOWN ? (
          <Row
            label={`All ${plain.length} chats`}
            small
            muted
            onPress={() => go({ pathname: '/project/[id]', params: { id: CHATS_ID, name: 'Chats' } })}
          />
        ) : null}

        <View style={styles.sectionRow}>
          <Text style={[type.labelMedium, styles.section]}>PROJECTS</Text>
          <IconButton icon="folder-plus-outline" label="Add project" onPress={() => setAdding(true)} size={18} />
        </View>
        {projects === null ? <ActivityIndicator color={m3.primary} style={{ marginTop: space.lg }} /> : null}
        {projects?.length === 0 ? <Text style={[type.bodyMedium, styles.empty]}>Add a project to start.</Text> : null}
        {projects?.map((p) => {
          const expanded = !!open[p.id];
          const list = chats[p.id] ?? [];
          const where = locationLabel(p.location, here);
          return (
            <View key={p.id}>
              <Row
                icon={expanded ? 'folder-open-outline' : 'folder-outline'}
                label={p.name}
                where={where && !where.brain ? where : null}
                active={activeProject === p.id}
                onPress={() => setOpen((o) => ({ ...o, [p.id]: !expanded }))}
                trailing={
                  <IconButton
                    icon="dots-horizontal"
                    label={`Open ${p.name}`}
                    size={18}
                    onPress={() => go({ pathname: '/project/[id]', params: { id: p.id, name: p.name } })}
                  />
                }
              />
              {expanded ? (
                <View style={styles.chats}>
                  {list.length === 0 ? <Text style={[type.bodySmall, styles.noChats]}>No chats yet</Text> : null}
                  {list.slice(0, CHATS_SHOWN).map((s) => (
                    <Row
                      key={s.id}
                      label={s.title}
                      small
                      active={activeSession === s.id}
                      status={s.status}
                      onPress={() => go({ pathname: '/session/[id]', params: { id: s.id, title: s.title } })}
                    />
                  ))}
                  {list.length > CHATS_SHOWN ? (
                    <Row
                      label={`All ${list.length} chats`}
                      small
                      muted
                      onPress={() => go({ pathname: '/project/[id]', params: { id: p.id, name: p.name } })}
                    />
                  ) : null}
                </View>
              ) : null}
            </View>
          );
        })}
      </ScrollView>

      <View style={styles.footer}>
        <Pressable
          onPress={() => go('/settings')}
          style={({ hovered }: { pressed: boolean; hovered?: boolean }) => [styles.status, hovered && styles.hover]}
          accessibilityRole="button"
          accessibilityLabel="Model status and settings"
        >
          <View
            style={[
              styles.dot,
              {
                backgroundColor: !model
                  ? m3.error
                  : model.state === 'ready'
                    ? m3.success
                    : model.state === 'idle'
                      ? m3.outline
                      : model.state === 'error'
                        ? m3.error
                        : m3.warning,
              },
            ]}
          />
          <View style={styles.grow}>
            <Text style={type.labelLarge} numberOfLines={1}>
              {!model
                ? 'Server unreachable'
                : model.state === 'ready'
                  ? 'Selected model'
                  : model.state === 'idle'
                    ? 'Model unloaded'
                    : model.state === 'error'
                      ? 'Model error'
                      : 'Model loading…'}
            </Text>
            {model?.model_path ? (
              <Text style={type.bodySmall} numberOfLines={1}>
                {model.model_path.split(/[\\/]/).pop()}
              </Text>
            ) : null}
          </View>
          <Icon name="cog-outline" size={20} />
        </Pressable>
      </View>

      <AddProjectSheet
        visible={adding}
        onClose={() => setAdding(false)}
        onCreate={async (name, path, deviceId) => {
          if (!client) return;
          const p = await client.createProject(name, path, deviceId);
          setAdding(false);
          setOpen((o) => ({ ...o, [p.id]: true }));
          emit('projects');
        }}
      />
    </View>
  );
}

function Row({
  label,
  icon,
  onPress,
  active,
  small,
  muted,
  status,
  trailing,
  where,
}: {
  where?: { text: string; online: boolean } | null;
  label: string;
  icon?: Parameters<typeof Icon>[0]['name'];
  onPress: () => void;
  active?: boolean;
  small?: boolean;
  muted?: boolean;
  status?: Session['status'];
  trailing?: React.ReactNode;
}) {
  const busy = status === 'running';
  const flagged = status === 'error';
  return (
    <View style={[styles.rowWrap, active && styles.active]}>
      <Pressable
        onPress={onPress}
        style={({ pressed, hovered }: { pressed: boolean; hovered?: boolean }) => [
          styles.row,
          small && styles.rowSmall,
          (hovered || pressed) && !active && styles.hover,
        ]}
        accessibilityRole="button"
        accessibilityLabel={where ? `${label}, files on ${where.text}${where.online ? '' : ' (offline)'}` : label}
        accessibilityState={{ selected: !!active }}
      >
        {icon ? <Icon name={icon} size={20} color={active ? m3.onSecondaryContainer : m3.onSurfaceVariant} /> : null}
        <View style={styles.grow}>
          <Text
            style={[
              small ? type.bodyMedium : type.labelLarge,
              {
                color: active ? m3.onSecondaryContainer : muted ? m3.outline : small ? m3.onSurfaceVariant : m3.onSurface,
              },
            ]}
            numberOfLines={1}
          >
            {label}
          </Text>
          {busy ? (
            <Text style={[type.bodySmall, styles.working]} numberOfLines={1}>
              Working…
            </Text>
          ) : null}
          {where ? (
            <View style={styles.where}>
              <Icon name="laptop" size={13} color={m3.onSurfaceVariant} />
              <Text style={[type.bodySmall, styles.whereText]} numberOfLines={1}>
                {where.text}
              </Text>
              <View style={[styles.dot, { backgroundColor: where.online ? m3.success : m3.outline }]} />
            </View>
          ) : null}
        </View>
        {busy ? <ActivityIndicator size="small" color={statusColor.running} /> : null}
        {flagged ? <View style={[styles.dot, { backgroundColor: m3.error }]} /> : null}
      </Pressable>
      {trailing}
    </View>
  );
}

const styles = StyleSheet.create({
  working: { color: m3.primary },
  side: {
    width: 288,
    backgroundColor: m3.surfaceContainerLow,
    paddingTop: space.md,
    paddingHorizontal: space.md,
  },
  grow: { flex: 1, minWidth: 0 },
  brand: { flexDirection: 'row', alignItems: 'center', gap: space.md, paddingLeft: space.sm, height: 48 },
  newChat: {
    flexDirection: 'row',
    alignItems: 'center',
    gap: space.md,
    height: 48,
    borderRadius: radius.lg,
    paddingHorizontal: space.lg,
    backgroundColor: m3.surfaceContainerHigh,
    marginTop: space.md,
  },
  newChatHover: { backgroundColor: m3.surfaceContainerHighest },
  kbd: { marginLeft: 'auto', color: m3.outline },
  sectionRow: {
    flexDirection: 'row',
    alignItems: 'center',
    justifyContent: 'space-between',
    minHeight: 36,
    marginTop: space.lg,
    paddingLeft: space.md,
  },
  section: { color: m3.onSurfaceVariant, letterSpacing: 1 },
  list: { paddingBottom: space.lg },
  homeRow: { marginTop: space.sm },
  brainRow: { marginTop: space.md },
  where: { flexDirection: 'row', alignItems: 'center', gap: 4 },
  whereText: { color: m3.onSurfaceVariant, flexShrink: 1 },
  empty: { paddingHorizontal: space.md, paddingVertical: space.sm },
  rowWrap: { flexDirection: 'row', alignItems: 'center', borderRadius: radius.md },
  row: {
    flex: 1,
    minWidth: 0,
    flexDirection: 'row',
    alignItems: 'center',
    gap: space.md,
    minHeight: 40,
    borderRadius: radius.md,
    paddingLeft: space.md,
    paddingRight: space.sm,
  },
  rowSmall: { minHeight: 34, paddingRight: space.md },
  chats: {
    marginLeft: 22,
    paddingLeft: space.sm,
    borderLeftWidth: 1,
    borderLeftColor: m3.outlineVariant,
    marginBottom: space.xs,
  },
  noChats: { paddingHorizontal: space.md, paddingVertical: 6, color: m3.outline },
  hover: { backgroundColor: m3.hover },
  active: { backgroundColor: m3.secondaryContainer },
  footer: { borderTopWidth: 1, borderTopColor: m3.outlineVariant, paddingVertical: space.sm },
  status: {
    flexDirection: 'row',
    alignItems: 'center',
    gap: space.md,
    borderRadius: radius.md,
    paddingHorizontal: space.md,
    paddingVertical: space.sm,
  },
  dot: { width: 8, height: 8, borderRadius: 4 },
});
