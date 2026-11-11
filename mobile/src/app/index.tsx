import { Redirect, router, useFocusEffect } from 'expo-router';
import { useCallback, useEffect, useRef, useState } from 'react';
import {
  ActivityIndicator,
  Image,
  Pressable,
  RefreshControl,
  ScrollView,
  StyleSheet,
  Text,
  TextInput,
  View,
} from 'react-native';
import { useSafeAreaInsets } from 'react-native-safe-area-context';

import { AddProjectSheet } from '../components/AddProjectSheet';
import { alert } from '../components/Dialogs';
import { AttachMenu, AttachmentStrip, useAttachments } from '../components/Attachments';
import { ModelChip, ModelSheet, modelLabel } from '../components/ModelPicker';
import { UpdateCard } from '../components/UpdateCard';
import { m3, radius, space, type } from '../components/theme';
import { BrainBanner, LocationChip } from '../components/Location';
import { Avatar, Banner, Button, Icon, IconButton, Logo, StatusBadge } from '../components/ui';
import { describeError } from '../lib/api';
import { useConnection } from '../lib/connection';
import { emit, on } from '../lib/events';
import { relativeTime } from '../lib/format';
import { enterToSend, useWide } from '../lib/layout';
import { autoRow, useRouting } from '../lib/routing';
import { loadLastTarget, saveLastTarget } from '../lib/storage';
import { CHATS_ID, type Project, type Session } from '../lib/types';
import { voiceAvailable } from '../lib/voice';

const CHATS_SHOWN = 12;

function greeting(): string {
  const h = new Date().getHours();
  if (h < 5) return 'Up late?';
  if (h < 12) return 'Good morning';
  if (h < 18) return 'Good afternoon';
  return 'Good evening';
}

/**
 * The home screen, on the phone and the desktop app: start a chat (plain, or
 * in a project), then every recent chat and every project below.
 */
export default function Home() {
  const { loading, client } = useConnection();
  const wide = useWide();
  const insets = useSafeAreaInsets();
  const [projects, setProjects] = useState<Project[] | null>(null);
  const [chats, setChats] = useState<Session[] | null>(null);
  const [loaded, setLoaded] = useState<string | null>(null);
  const routing = useRouting();
  /** Model picked for the next chat; null follows the project or what's loaded. */
  const [model, setModel] = useState<string | null>(null);
  const [pickingModel, setPickingModel] = useState(false);
  const [starting, setStarting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const pics = useAttachments();
  const [target, setTarget] = useState<string>(CHATS_ID);
  /** Projects as last loaded, so a remembered pick can be checked against them. */
  const known = useRef<Project[] | null>(null);
  /** True once the user picks here, so a late restore doesn't override them. */
  const chosen = useRef(false);
  const [picking, setPicking] = useState(false);
  const [adding, setAdding] = useState(false);
  const [text, setText] = useState('');
  const [busy, setBusy] = useState(false);
  const [refreshing, setRefreshing] = useState(false);
  /** Chats ("c:id") and projects ("p:id") picked with a long press. */
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [acting, setActing] = useState(false);
  const input = useRef<React.ComponentRef<typeof TextInput>>(null);
  /** Queued and running background tasks (the badge on Tasks). */
  const [taskCount, setTaskCount] = useState(0);

  const load = useCallback(async () => {
    if (!client) return;
    try {
      const [ps, cs, m] = await Promise.all([
        client.projects(),
        client.sessions(CHATS_ID),
        client.model().catch(() => null),
      ]);
      known.current = ps;
      setProjects(ps);
      setChats(cs);
      client
        .queue()
        .then((q) => setTaskCount(q.queued + q.running))
        .catch(() => setTaskCount(0));
      setLoaded(m?.default_model || m?.model_path || null);
      setTarget((cur) => (cur === CHATS_ID || ps.some((p) => p.id === cur) ? cur : CHATS_ID));
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
  // Pick the project from last time. Until the projects arrive it's taken on
  // trust; load() falls back to a plain chat if that project is gone.
  useEffect(() => {
    let live = true;
    loadLastTarget().then((id) => {
      if (!live || !id || chosen.current) return;
      const ps = known.current;
      if (id === CHATS_ID || !ps || ps.some((p) => p.id === id)) setTarget(id);
    });
    return () => {
      live = false;
    };
  }, []);
  useEffect(() => {
    const a = on('projects', load);
    const b = on('chats', load);
    return () => {
      a();
      b();
    };
  }, [load]);
  // While a chat is working, check back so its "Working…" line clears.
  const anyWorking = !!chats?.some((c) => c.status === 'running');
  useFocusEffect(
    useCallback(() => {
      if (!anyWorking) return;
      const t = setInterval(load, 4000);
      return () => clearInterval(t);
    }, [anyWorking, load]),
  );

  if (loading) {
    return (
      <View style={styles.center}>
        <ActivityIndicator color={m3.primary} />
      </View>
    );
  }
  if (!client) return <Redirect href="/connect" />;

  const project = projects?.find((p) => p.id === target) ?? null;
  const canSend = (!!text.trim() || pics.items.length > 0) && !busy;
  const canTalk = voiceAvailable();
  const open = (href: Parameters<typeof router.push>[0]) => (wide ? router.replace(href) : router.push(href));

  const send = async () => {
    if ((!text.trim() && !pics.items.length) || busy) return;
    setBusy(true);
    setError(null);
    try {
      const prompt = text.trim();
      // No title: Relay names the chat after the first reply.
      const s = await client.createSession(project?.id ?? CHATS_ID);
      if (model) await client.setSessionModel(s.id, model);
      await client.sendPrompt(
        s.id,
        prompt,
        pics.images,
        pics.files,
      );
      setModel(null);
      setText('');
      pics.clear();
      emit('chats');
      open({ pathname: '/session/[id]', params: { id: s.id, title: s.title } });
    } catch (e) {
      setError(describeError(e));
    } finally {
      setBusy(false);
    }
  };

  /** A new chat in the chosen place, opened in voice mode. */
  const talk = async () => {
    if (busy) return;
    setBusy(true);
    setError(null);
    try {
      const s = await client.createSession(project?.id ?? CHATS_ID);
      if (model) await client.setSessionModel(s.id, model);
      setModel(null);
      emit('chats');
      open({ pathname: '/session/[id]', params: { id: s.id, title: s.title, voice: '1' } });
    } catch (e) {
      setError(describeError(e));
    } finally {
      setBusy(false);
    }
  };

  /** Adds the prompt to the background queue instead of starting it now. */
  const queue = async () => {
    if (!text.trim() || busy) return;
    setBusy(true);
    setError(null);
    try {
      const item = await client.enqueue({ prompt: text.trim(), project_id: project?.id ?? CHATS_ID });
      setText('');
      setTaskCount((n) => n + 1);
      alert('Added to the queue', `“${item.name}” runs on your PC when the tasks before it are done. See it under Tasks.`, [
        { text: 'OK', style: 'cancel' },
        { text: 'Open Tasks', onPress: () => open('/tasks') },
      ]);
    } catch (e) {
      setError(describeError(e));
    } finally {
      setBusy(false);
    }
  };

  /** An empty plain chat, opened right away. */
  const newChat = async () => {
    setStarting(true);
    try {
      const s = await client.createSession(CHATS_ID);
      emit('chats');
      open({ pathname: '/session/[id]', params: { id: s.id, title: s.title } });
    } catch (e) {
      setError(describeError(e));
    } finally {
      setStarting(false);
    }
  };

  const choose = (id: string) => {
    chosen.current = true;
    setTarget(id);
    saveLastTarget(id);
    setPicking(false);
    input.current?.focus();
  };

  const selecting = selected.size > 0;
  const toggle = (key: string) =>
    setSelected((cur) => {
      const next = new Set(cur);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });
  const picked = () => {
    const ids = [...selected];
    return {
      chats: ids.filter((k) => k.startsWith('c:')).map((k) => k.slice(2)),
      projects: ids.filter((k) => k.startsWith('p:')).map((k) => k.slice(2)),
    };
  };
  const selectionLabel = () => {
    const { chats: c, projects: p } = picked();
    const parts = [];
    if (c.length) parts.push(`${c.length} chat${c.length === 1 ? '' : 's'}`);
    if (p.length) parts.push(`${p.length} project${p.length === 1 ? '' : 's'}`);
    return parts.join(' and ');
  };
  /** Runs `act` on every selected chat and project, then reports any that failed. */
  const bulk = async (
    verb: string,
    onChat: (id: string) => Promise<unknown>,
    onProject: (id: string) => Promise<unknown>,
  ) => {
    const { chats: c, projects: p } = picked();
    setActing(true);
    const results = await Promise.allSettled([...c.map(onChat), ...p.map(onProject)]);
    setActing(false);
    const failed = results.filter((r): r is PromiseRejectedResult => r.status === 'rejected');
    setSelected(new Set());
    if (p.some((id) => id === target)) choose(CHATS_ID);
    emit('chats');
    emit('projects');
    load();
    if (failed.length) {
      alert(`Could not ${verb} ${failed.length} of them`, describeError(failed[0].reason));
    }
  };
  const archiveSelected = () =>
    bulk(
      'archive',
      (id) => client.archiveSession(id, true),
      (id) => client.archiveProject(id, true),
    );
  const deleteSelected = () => {
    const { projects: p } = picked();
    alert(
      `Delete ${selectionLabel()}?`,
      p.length
        ? 'Chats are deleted for good. Projects are removed from Relay with all their chats, but their folders and files on your PC are kept.'
        : 'These chats are deleted for good.',
      [
        { text: 'Cancel', style: 'cancel' },
        {
          text: 'Delete',
          style: 'destructive',
          onPress: () =>
            bulk(
              'delete',
              (id) => client.deleteSession(id),
              (id) => client.deleteProject(id),
            ),
        },
      ],
    );
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
              if (target === p.id) choose(CHATS_ID);
              emit('projects');
              load();
            } catch (e) {
              alert('Could not remove project', describeError(e));
            }
          },
        },
      ],
    );
  };

  const menuItem = (key: string, icon: Parameters<typeof Icon>[0]['name'], label: string, sub?: string) => (
    <Pressable
      key={key}
      onPress={() => choose(key)}
      style={({ hovered, pressed }: { pressed: boolean; hovered?: boolean }) => [
        styles.menuItem,
        (hovered || pressed) && styles.hover,
      ]}
      accessibilityRole="menuitem"
      accessibilityState={{ selected: target === key }}
    >
      <Icon
        name={target === key ? 'check' : icon}
        size={18}
        color={target === key ? m3.primary : m3.onSurfaceVariant}
      />
      <View style={styles.grow}>
        <Text style={type.bodyLarge} numberOfLines={1}>
          {label}
        </Text>
        {sub ? (
          <Text style={type.bodySmall} numberOfLines={1}>
            {sub}
          </Text>
        ) : null}
      </View>
    </Pressable>
  );

  return (
    <View style={[styles.flex, !wide && { paddingTop: insets.top }]}>
      {!wide ? (
        <View style={styles.appBar}>
          <View style={styles.brand}>
            <Logo size={32} />
            <Text style={type.titleLarge}>Relay</Text>
          </View>
          <View style={styles.appBarActions}>
            <View>
              <IconButton
                icon="calendar-clock"
                label={taskCount ? `Tasks, ${taskCount} running or waiting` : 'Tasks'}
                onPress={() => router.push('/tasks')}
              />
              {taskCount ? (
                <View style={styles.badge} pointerEvents="none">
                  <Text style={[type.labelMedium, { color: m3.onPrimary, fontSize: 11 }]}>{taskCount}</Text>
                </View>
              ) : null}
            </View>
            <IconButton icon="cog-outline" label="Settings" onPress={() => router.push('/settings')} />
          </View>
        </View>
      ) : null}
      <ScrollView
        style={styles.flex}
        contentContainerStyle={[
          styles.content,
          wide && styles.contentWide,
          { paddingBottom: insets.bottom + (selecting ? 120 : 48) },
        ]}
        keyboardShouldPersistTaps="handled"
        refreshControl={
          wide ? undefined : (
            <RefreshControl
              refreshing={refreshing}
              onRefresh={async () => {
                setRefreshing(true);
                await load();
                setRefreshing(false);
              }}
              tintColor={m3.onSurfaceVariant}
            />
          )
        }
      >
        <View style={styles.column}>
          <View style={[styles.hello, !wide && styles.helloPhone]}>
            {wide ? <Logo size={40} /> : null}
            <Text style={[type.displaySmall, wide && styles.greetingWide]}>{greeting()}</Text>
          </View>
          {!wide ? (
            <View style={styles.brainPhone}>
              <BrainBanner onPress={() => open('/devices')} />
            </View>
          ) : null}

          <View style={styles.composer}>
            <View style={styles.topRow}>
              <Pressable
                onPress={() => setPicking((v) => !v)}
                style={({ hovered, pressed }: { pressed: boolean; hovered?: boolean }) => [
                  styles.target,
                  picking && styles.targetOn,
                  (hovered || pressed) && !picking && styles.hover,
                ]}
                accessibilityRole="button"
                accessibilityLabel={`${project ? `Project: ${project.name}` : 'Plain chat'}. Change`}
                accessibilityState={{ expanded: picking }}
              >
                <Icon
                  name={project ? 'folder-outline' : 'chat-outline'}
                  size={18}
                  color={picking ? m3.onSecondaryContainer : m3.onSurfaceVariant}
                />
                <Text
                  style={[type.labelLarge, styles.targetLabel, picking && { color: m3.onSecondaryContainer }]}
                  numberOfLines={1}
                >
                  {project ? project.name : 'Chat'}
                </Text>
                <Icon
                  name={picking ? 'chevron-up' : 'chevron-down'}
                  size={16}
                  color={picking ? m3.onSecondaryContainer : m3.onSurfaceVariant}
                />
              </Pressable>
            </View>
            <AttachmentStrip items={pics.items} onRemove={pics.remove} />
            <TextInput
              ref={input}
              style={[styles.input, !wide && styles.inputPhone]}
              value={text}
              onChangeText={setText}
              onKeyPress={enterToSend(send)}
              placeholder={project ? `What should we do in ${project.name}?` : 'Ask anything'}
              placeholderTextColor={m3.outline}
              selectionColor={m3.primary}
              multiline
              autoFocus={wide}
              accessibilityLabel="Prompt"
            />
            <View style={styles.row}>
              <View style={styles.modelSlot}>
                <AttachMenu onPick={pics.pick} disabled={busy} />
                <ModelChip
                  model={model}
                  fallback={project?.model ?? loaded}
                  auto={!!routing && !project?.model}
                  onPress={() => setPickingModel(true)}
                />
              </View>
              <IconButton
                icon="tray-plus"
                label="Add to queue instead: it runs when earlier tasks are done"
                onPress={queue}
                disabled={!text.trim() || pics.items.length > 0 || busy}
              />
              {canTalk && !canSend ? (
                <Pressable
                  onPress={talk}
                  disabled={busy}
                  style={[styles.send, styles.sendOn]}
                  accessibilityRole="button"
                  accessibilityLabel="Start a voice chat"
                >
                  {busy ? (
                    <ActivityIndicator color={m3.onPrimary} />
                  ) : (
                    <Icon name="account-voice" size={22} color={m3.onPrimary} />
                  )}
                </Pressable>
              ) : (
              <Pressable
                onPress={send}
                disabled={!canSend}
                style={[styles.send, canSend ? styles.sendOn : styles.sendOff]}
                accessibilityRole="button"
                accessibilityLabel="Start chat"
              >
                {busy ? (
                  <ActivityIndicator color={m3.onPrimary} />
                ) : (
                  <Icon name="arrow-up" size={22} color={canSend ? m3.onPrimary : m3.outline} />
                )}
              </Pressable>
              )}
            </View>

            {picking ? (
              <View style={[styles.menu, !wide && styles.menuPhone]}>
                {menuItem(CHATS_ID, 'chat-outline', 'Chat', 'Ask anything. No files or code')}
                {projects?.length ? <View style={styles.menuDivider} /> : null}
                {projects?.map((p) => menuItem(p.id, 'folder-outline', p.name))}
                <View style={styles.menuDivider} />
                <Pressable
                  onPress={() => {
                    setPicking(false);
                    setAdding(true);
                  }}
                  style={({ hovered, pressed }: { pressed: boolean; hovered?: boolean }) => [
                    styles.menuItem,
                    (hovered || pressed) && styles.hover,
                  ]}
                  accessibilityRole="menuitem"
                >
                  <Icon name="folder-plus-outline" size={18} color={m3.primary} />
                  <Text style={[type.labelLarge, { color: m3.primary }]}>New project…</Text>
                </Pressable>
              </View>
            ) : null}
          </View>

          <Text style={[type.bodySmall, styles.hint]}>
            {project
              ? 'The agent can read, edit and run code in this project.'
              : 'A plain chat. It has no access to your files.'}
            {wide ? ' · Enter to send · Shift+Enter for a new line' : ''}
          </Text>

          {error ? <Banner message={error} actionLabel="Retry" onAction={load} /> : null}
          <UpdateCard client={client} onlyWhenAvailable />
          {pics.error ? <Banner kind="warning" message={pics.error} actionLabel="Dismiss" onAction={() => pics.setError(null)} /> : null}

          <View style={styles.sectionRow}>
            <Text style={[type.titleSmall, styles.section]}>Chats</Text>
            <Button title="New chat" icon="plus" variant="tonal" onPress={newChat} loading={starting} />
          </View>
          {chats === null && !error ? <ActivityIndicator color={m3.primary} /> : null}
          {chats?.length === 0 ? (
            <Text style={[type.bodyMedium, styles.empty]}>
              No chats yet. Ask something above, or tap New chat. Plain chats can&apos;t see your files.
            </Text>
          ) : null}
          {chats?.length ? (
            <View style={styles.group}>
              {chats.slice(0, CHATS_SHOWN).map((s, i) => (
                <View key={s.id}>
                  {i > 0 ? <View style={styles.divider} /> : null}
                  <Pressable
                    onPress={() =>
                      selecting
                        ? toggle(`c:${s.id}`)
                        : open({ pathname: '/session/[id]', params: { id: s.id, title: s.title } })
                    }
                    onLongPress={() => toggle(`c:${s.id}`)}
                    style={({ hovered, pressed }: { pressed: boolean; hovered?: boolean }) => [
                      styles.item,
                      (hovered || pressed) && styles.hover,
                      selected.has(`c:${s.id}`) && styles.selected,
                    ]}
                    accessibilityRole={selecting ? 'checkbox' : 'button'}
                    accessibilityState={selecting ? { checked: selected.has(`c:${s.id}`) } : undefined}
                    accessibilityLabel={s.title}
                    accessibilityHint={selecting ? undefined : 'Opens the chat. Press and hold to select.'}
                  >
                    <Icon
                      name={selected.has(`c:${s.id}`) ? 'check-circle' : selecting ? 'circle-outline' : 'chat-outline'}
                      size={20}
                      color={selected.has(`c:${s.id}`) ? m3.primary : m3.onSurfaceVariant}
                    />
                    <View style={styles.grow}>
                      <Text style={type.bodyLarge} numberOfLines={1}>
                        {s.title}
                      </Text>
                      {s.status === 'running' ? (
                        <Text style={[type.bodySmall, styles.working]} numberOfLines={1}>
                          Working…
                        </Text>
                      ) : (
                        <Text style={type.bodySmall} numberOfLines={1}>
                          {relativeTime(s.updated_at)}
                        </Text>
                      )}
                    </View>
                    {s.status === 'error' ? <StatusBadge status={s.status} /> : null}
                  </Pressable>
                </View>
              ))}
              {chats.length > CHATS_SHOWN ? (
                <>
                  <View style={styles.divider} />
                  <Pressable
                    onPress={() => open({ pathname: '/project/[id]', params: { id: CHATS_ID, name: 'Chats' } })}
                    style={({ hovered, pressed }: { pressed: boolean; hovered?: boolean }) => [
                      styles.item,
                      (hovered || pressed) && styles.hover,
                    ]}
                    accessibilityRole="button"
                  >
                    <Text style={[type.labelLarge, styles.link, styles.moreText]}>All {chats.length} chats</Text>
                  </Pressable>
                </>
              ) : null}
            </View>
          ) : null}

          <View style={styles.sectionRow}>
            <Text style={[type.titleSmall, styles.section]}>Projects</Text>
            <Button title="New project" icon="folder-plus-outline" variant="tonal" onPress={() => setAdding(true)} />
          </View>
          {projects?.length === 0 ? (
            <Text style={[type.bodyMedium, styles.empty]}>
              A project is a folder the agent can work in, on the brain or on any computer linked to it.
            </Text>
          ) : null}
          {projects?.length ? (
            <View style={styles.group}>
              {projects.map((p, i) => (
                <View key={p.id}>
                  {i > 0 ? <View style={styles.divider} /> : null}
                  <View style={styles.projectRow}>
                    <Pressable
                      onPress={() =>
                        selecting
                          ? toggle(`p:${p.id}`)
                          : open({ pathname: '/project/[id]', params: { id: p.id, name: p.name } })
                      }
                      onLongPress={() => toggle(`p:${p.id}`)}
                      style={({ hovered, pressed }: { pressed: boolean; hovered?: boolean }) => [
                        styles.item,
                        styles.grow,
                        (hovered || pressed) && styles.hover,
                        selected.has(`p:${p.id}`) && styles.selected,
                      ]}
                      accessibilityRole={selecting ? 'checkbox' : 'button'}
                      accessibilityState={selecting ? { checked: selected.has(`p:${p.id}`) } : undefined}
                      accessibilityLabel={p.name}
                      accessibilityHint={selecting ? undefined : 'Opens its chats. Press and hold to select.'}
                    >
                      {selecting ? (
                        <View style={styles.checkSlot}>
                          <Icon
                            name={selected.has(`p:${p.id}`) ? 'check-circle' : 'circle-outline'}
                            size={22}
                            color={selected.has(`p:${p.id}`) ? m3.primary : m3.onSurfaceVariant}
                          />
                        </View>
                      ) : (
                        <Avatar label={p.name} size={32} />
                      )}
                      <View style={styles.grow}>
                        <Text style={type.bodyLarge} numberOfLines={1}>
                          {p.name}
                        </Text>
                        <LocationChip location={p.location} compact />
                      </View>
                    </Pressable>
                    {selecting ? null : (
                      <IconButton icon="dots-vertical" label={`Remove ${p.name}`} onPress={() => confirmDelete(p)} />
                    )}
                  </View>
                </View>
              ))}
            </View>
          ) : null}

          <Pressable
            onPress={() => open('/archived')}
            style={({ hovered, pressed }: { pressed: boolean; hovered?: boolean }) => [
              styles.archivedLink,
              (hovered || pressed) && styles.hover,
            ]}
            accessibilityRole="button"
          >
            <Icon name="archive-outline" size={18} color={m3.onSurfaceVariant} />
            <Text style={[type.labelLarge, { color: m3.onSurfaceVariant }]}>Archived</Text>
          </Pressable>

          {wide ? <PairCard /> : null}
        </View>
      </ScrollView>

      {selecting ? (
        <View style={[styles.selectBar, { bottom: insets.bottom + space.lg }]}>
          <IconButton icon="close" label="Cancel selection" onPress={() => setSelected(new Set())} />
          <Text style={[type.titleSmall, styles.grow]} numberOfLines={1}>
            {wide ? selectionLabel() : `${selected.size} selected`}
          </Text>
          {acting ? <ActivityIndicator color={m3.primary} /> : null}
          {wide ? (
            <>
              <Button
                title="Archive"
                icon="archive-outline"
                variant="tonal"
                onPress={archiveSelected}
                disabled={acting}
              />
              <Button title="Delete" icon="delete-outline" variant="tonal" onPress={deleteSelected} disabled={acting} />
            </>
          ) : (
            <>
              <IconButton icon="archive-outline" label="Archive" onPress={archiveSelected} disabled={acting} />
              <IconButton icon="delete-outline" label="Delete" onPress={deleteSelected} disabled={acting} />
            </>
          )}
        </View>
      ) : null}

      <ModelSheet
        visible={pickingModel}
        onClose={() => setPickingModel(false)}
        value={model}
        defaultTitle={project?.model ? 'Same as the project' : routing ? 'Auto' : 'The default model'}
        defaultBody={
          project?.model
            ? `${project.name}: ${modelLabel(project.model)}`
            : routing
              ? autoRow(routing, loaded).body
              : loaded
                ? `Now: ${modelLabel(loaded)}`
                : 'The model chosen in Settings'
        }
        onPick={async (path) => setModel(path || null)}
      />
      <AddProjectSheet
        visible={adding}
        onClose={() => setAdding(false)}
        onCreate={async (name, path, deviceId) => {
          const p = await client.createProject(name, path, deviceId);
          setAdding(false);
          choose(p.id);
          emit('projects');
          load();
        }}
      />
    </View>
  );
}

/** Small pairing QR on the home screen; opens Settings for the larger code and details. */
function PairCard() {
  const { client } = useConnection();
  const [svg, setSvg] = useState<string | null>(null);
  // undefined while loading, so the card doesn't flash for paired phones.
  const [phoneSeen, setPhoneSeen] = useState<string | null | undefined>(undefined);
  useEffect(() => {
    client
      ?.info()
      .then((i) => setPhoneSeen(i.phone_last_seen ?? null))
      .catch(() => setPhoneSeen(null));
    client
      ?.pairing()
      .then((p) => setSvg(p.svg))
      .catch(() => setSvg(null));
  }, [client]);
  if (phoneSeen === undefined) return null;
  if (phoneSeen) {
    return (
      <Pressable
        onPress={() => router.replace('/settings')}
        style={styles.phoneOk}
        accessibilityRole="button"
        accessibilityLabel="Phone connected. Opens Settings to pair another phone."
      >
        <View style={[styles.dot, { backgroundColor: m3.success }]} />
        <Text style={[type.bodySmall, styles.pairBody]}>Phone connected · last seen {relativeTime(phoneSeen)}</Text>
      </Pressable>
    );
  }
  return (
    <Pressable
      onPress={() => router.replace('/settings')}
      style={({ hovered }: { pressed: boolean; hovered?: boolean }) => [styles.pair, hovered && styles.pairHover]}
      accessibilityRole="button"
      accessibilityLabel="Pair your phone. Opens Settings with a larger code."
    >
      <View style={styles.pairQr}>
        {svg ? (
          <Image
            source={{ uri: `data:image/svg+xml;base64,${globalThis.btoa(svg)}` }}
            style={styles.pairQrImage}
            accessibilityLabel="Pairing QR code"
          />
        ) : (
          <Icon name="qrcode" size={40} color={m3.surfaceContainerHighest} />
        )}
      </View>
      <View style={styles.pairText}>
        <View style={styles.pairTitle}>
          <Icon name="cellphone-link" size={18} color={m3.primary} />
          <Text style={type.titleSmall}>Pair your phone</Text>
        </View>
        <Text style={[type.bodySmall, styles.pairBody]}>
          In the Relay app on your phone, tap Scan QR code and point it at this code.
        </Text>
        <Text style={[type.labelMedium, styles.pairLink]}>Bigger code and details</Text>
      </View>
    </Pressable>
  );
}

const styles = StyleSheet.create({
  working: { color: m3.primary },
  brainPhone: { marginBottom: space.md },
  flex: { flex: 1, backgroundColor: m3.surface },
  center: { flex: 1, alignItems: 'center', justifyContent: 'center', backgroundColor: m3.surface },
  grow: { flex: 1, minWidth: 0 },
  appBar: {
    height: 64,
    flexDirection: 'row',
    alignItems: 'center',
    justifyContent: 'space-between',
    paddingLeft: space.lg,
    paddingRight: space.xs,
  },
  brand: { flexDirection: 'row', alignItems: 'center', gap: space.md },
  appBarActions: { flexDirection: 'row', alignItems: 'center' },
  badge: {
    position: 'absolute',
    top: 4,
    right: 4,
    minWidth: 18,
    height: 18,
    borderRadius: 9,
    paddingHorizontal: 4,
    alignItems: 'center',
    justifyContent: 'center',
    backgroundColor: m3.primary,
  },
  content: { padding: space.lg },
  contentWide: { paddingHorizontal: space.xl, paddingTop: 72 },
  column: { width: '100%', maxWidth: 720, alignSelf: 'center', gap: space.md },
  hello: {
    flexDirection: 'row',
    alignItems: 'center',
    justifyContent: 'center',
    gap: space.lg,
    marginBottom: space.md,
  },
  helloPhone: { justifyContent: 'flex-start', marginHorizontal: space.xs, marginBottom: space.xs },
  greetingWide: { fontSize: 40, lineHeight: 48 },
  composer: {
    backgroundColor: m3.surfaceContainerHigh,
    borderRadius: radius.xl,
    padding: space.md,
    zIndex: 10,
    borderWidth: 1,
    borderColor: m3.outlineVariant,
  },
  input: {
    minHeight: 56,
    maxHeight: 320,
    color: m3.onSurface,
    fontSize: 17,
    lineHeight: 25,
    paddingHorizontal: space.md,
    paddingTop: space.sm,
  },
  inputPhone: { minHeight: 44, fontSize: 16, lineHeight: 22 },
  row: { flexDirection: 'row', alignItems: 'flex-end', gap: space.sm, marginTop: space.sm },
  topRow: { flexDirection: 'row', alignItems: 'center' },
  target: {
    flexDirection: 'row',
    alignItems: 'center',
    gap: 6,
    height: 36,
    flexShrink: 1,
    paddingLeft: space.md,
    paddingRight: space.sm,
    borderRadius: 18,
    borderWidth: 1,
    borderColor: m3.outlineVariant,
  },
  targetLabel: { flexShrink: 1, color: m3.onSurfaceVariant },
  targetOn: { backgroundColor: m3.secondaryContainer, borderColor: m3.secondaryContainer },
  modelSlot: { flex: 1, minWidth: 0, flexDirection: 'row', alignItems: 'center', gap: 4 },
  hover: { backgroundColor: m3.hover },
  selected: { backgroundColor: m3.secondaryContainer },
  checkSlot: { width: 32, alignItems: 'center' },
  archivedLink: {
    alignSelf: 'flex-start',
    flexDirection: 'row',
    alignItems: 'center',
    gap: space.sm,
    height: 40,
    paddingHorizontal: space.md,
    borderRadius: radius.sm,
  },
  selectBar: {
    position: 'absolute',
    left: space.lg,
    right: space.lg,
    alignSelf: 'center',
    maxWidth: 720,
    marginHorizontal: 'auto',
    flexDirection: 'row',
    alignItems: 'center',
    gap: space.sm,
    paddingLeft: space.xs,
    paddingRight: space.md,
    minHeight: 60,
    borderRadius: radius.xl,
    backgroundColor: m3.surfaceContainerHighest,
    borderWidth: 1,
    borderColor: m3.outlineVariant,
    shadowColor: '#000',
    shadowOpacity: 0.4,
    shadowRadius: 16,
    shadowOffset: { width: 0, height: 8 },
    elevation: 6,
  },
  send: { width: 44, height: 44, borderRadius: 22, alignItems: 'center', justifyContent: 'center' },
  sendOn: { backgroundColor: m3.primary },
  sendOff: { backgroundColor: m3.surfaceContainerHighest },
  menu: {
    position: 'absolute',
    top: space.md + 40,
    left: space.md,
    width: 340,
    maxWidth: '92%',
    backgroundColor: m3.surfaceContainer,
    borderRadius: radius.lg,
    paddingVertical: space.sm,
    borderWidth: 1,
    borderColor: m3.outlineVariant,
    shadowColor: '#000',
    shadowOpacity: 0.4,
    shadowRadius: 16,
    shadowOffset: { width: 0, height: 8 },
    elevation: 6,
  },
  menuPhone: { left: 0, right: 0, width: undefined, maxWidth: undefined },
  menuItem: {
    flexDirection: 'row',
    alignItems: 'center',
    gap: space.md,
    minHeight: 48,
    paddingHorizontal: space.lg,
    paddingVertical: space.xs,
  },
  menuDivider: { height: 1, backgroundColor: m3.outlineVariant, marginVertical: space.xs },
  hint: { color: m3.outline, textAlign: 'center' },
  sectionRow: {
    flexDirection: 'row',
    alignItems: 'center',
    justifyContent: 'space-between',
    marginTop: space.lg,
    marginHorizontal: space.xs,
  },
  section: { color: m3.onSurfaceVariant },
  link: { color: m3.primary },
  moreText: { marginLeft: 36 },
  empty: { color: m3.onSurfaceVariant, marginHorizontal: space.xs },
  group: { backgroundColor: m3.surfaceContainerLow, borderRadius: radius.xl, overflow: 'hidden' },
  item: {
    flexDirection: 'row',
    alignItems: 'center',
    gap: space.md,
    minHeight: 60,
    paddingHorizontal: space.lg,
    paddingVertical: space.sm,
  },
  projectRow: { flexDirection: 'row', alignItems: 'center', paddingRight: space.xs },
  divider: { height: StyleSheet.hairlineWidth, backgroundColor: m3.outlineVariant, marginLeft: 52 },
  pair: {
    alignSelf: 'center',
    flexDirection: 'row',
    alignItems: 'center',
    gap: space.lg,
    width: '100%',
    maxWidth: 440,
    padding: space.md,
    borderRadius: radius.lg,
    backgroundColor: m3.surfaceContainer,
    borderWidth: 1,
    borderColor: m3.outlineVariant,
    marginTop: space.lg,
  },
  pairHover: { backgroundColor: m3.surfaceContainerHigh },
  pairQr: {
    width: 96,
    height: 96,
    borderRadius: radius.sm,
    backgroundColor: '#FFFFFF',
    alignItems: 'center',
    justifyContent: 'center',
    padding: 4,
  },
  pairQrImage: { width: 88, height: 88 },
  pairText: { flex: 1, minWidth: 0, gap: space.xs },
  pairTitle: { flexDirection: 'row', alignItems: 'center', gap: space.sm },
  pairBody: { color: m3.onSurfaceVariant },
  pairLink: { color: m3.primary },
  phoneOk: { alignSelf: 'center', flexDirection: 'row', alignItems: 'center', gap: space.sm, marginTop: space.md },
  dot: { width: 8, height: 8, borderRadius: 4 },
});
