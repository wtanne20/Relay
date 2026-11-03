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
import { ModelStatusBar } from '../components/ModelStatusBar';
import { m3, radius, space, type } from '../components/theme';
import { Avatar, Banner, Icon, IconButton, Logo, StatusBadge } from '../components/ui';
import { describeError } from '../lib/api';
import { useConnection } from '../lib/connection';
import { emit, on } from '../lib/events';
import { relativeTime } from '../lib/format';
import { enterToSend, useWide } from '../lib/layout';
import { CHATS_ID, type Project, type RecentSession } from '../lib/types';

const LAST_TARGET = 'relay.lastTarget';
const RECENT_SHOWN = 12;

function greeting(): string {
  const h = new Date().getHours();
  if (h < 5) return 'Up late?';
  if (h < 12) return 'Good morning';
  if (h < 18) return 'Good afternoon';
  return 'Good evening';
}

function remembered(): string {
  try {
    return globalThis.localStorage?.getItem(LAST_TARGET) ?? CHATS_ID;
  } catch {
    return CHATS_ID;
  }
}

function remember(id: string) {
  try {
    globalThis.localStorage?.setItem(LAST_TARGET, id);
  } catch {
    // Not important: plain chat is picked next time.
  }
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
  const [recent, setRecent] = useState<RecentSession[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [target, setTarget] = useState<string>(remembered());
  const [picking, setPicking] = useState(false);
  const [adding, setAdding] = useState(false);
  const [text, setText] = useState('');
  const [busy, setBusy] = useState(false);
  const [focused, setFocused] = useState(false);
  const [refreshing, setRefreshing] = useState(false);
  const input = useRef<React.ComponentRef<typeof TextInput>>(null);

  const load = useCallback(async () => {
    if (!client) return;
    try {
      const [ps, rs] = await Promise.all([client.projects(), client.recent(RECENT_SHOWN)]);
      setProjects(ps);
      setRecent(rs);
      setTarget((cur) => (cur === CHATS_ID || ps.some((p) => p.id === cur) ? cur : CHATS_ID));
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
  useEffect(() => {
    const a = on('projects', load);
    const b = on('chats', load);
    return () => {
      a();
      b();
    };
  }, [load]);

  if (loading) {
    return (
      <View style={styles.center}>
        <ActivityIndicator color={m3.primary} />
      </View>
    );
  }
  if (!client) return <Redirect href="/connect" />;

  const project = projects?.find((p) => p.id === target) ?? null;
  const canSend = !!text.trim() && !busy;
  const open = (href: Parameters<typeof router.push>[0]) => (wide ? router.replace(href) : router.push(href));

  const send = async () => {
    if (!text.trim() || busy) return;
    setBusy(true);
    setError(null);
    try {
      const prompt = text.trim();
      // No title: Relay names the chat after the first reply.
      const s = await client.createSession(project?.id ?? CHATS_ID);
      await client.sendPrompt(s.id, prompt);
      setText('');
      emit('chats');
      open({ pathname: '/session/[id]', params: { id: s.id, title: s.title } });
    } catch (e) {
      setError(describeError(e));
    } finally {
      setBusy(false);
    }
  };

  const choose = (id: string) => {
    setTarget(id);
    remember(id);
    setPicking(false);
    input.current?.focus();
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
          <IconButton icon="cog-outline" label="Settings" onPress={() => router.push('/settings')} />
        </View>
      ) : null}
      <ScrollView
        style={styles.flex}
        contentContainerStyle={[styles.content, wide && styles.contentWide, { paddingBottom: insets.bottom + 48 }]}
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

          <View style={styles.composer}>
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
              <Pressable
                onPress={() => setPicking((v) => !v)}
                style={({ hovered }: { pressed: boolean; hovered?: boolean }) => [styles.chip, hovered && styles.hover]}
                accessibilityRole="button"
                accessibilityLabel={`${project ? `Project: ${project.name}` : 'Plain chat'}. Change`}
              >
                <Icon name={project ? 'folder-outline' : 'chat-outline'} size={18} color={m3.onSurfaceVariant} />
                <Text style={[type.labelLarge, { color: m3.onSurfaceVariant }]} numberOfLines={1}>
                  {project?.name ?? 'Chat'}
                </Text>
                <Icon name={picking ? 'chevron-up' : 'chevron-down'} size={18} color={m3.onSurfaceVariant} />
              </Pressable>
              <View style={styles.grow} />
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

          {!wide ? <ModelStatusBar client={client} active={focused} onPress={() => router.push('/settings')} /> : null}
          {error ? <Banner message={error} actionLabel="Retry" onAction={load} /> : null}

          <View style={styles.sectionRow}>
            <Text style={[type.titleSmall, styles.section]}>Recent chats</Text>
            <Pressable
              onPress={() => open({ pathname: '/project/[id]', params: { id: CHATS_ID, name: 'Chats' } })}
              hitSlop={8}
              accessibilityRole="button"
            >
              <Text style={[type.labelLarge, styles.link]}>All plain chats</Text>
            </Pressable>
          </View>
          {recent === null && !error ? <ActivityIndicator color={m3.primary} /> : null}
          {recent?.length === 0 ? (
            <Text style={[type.bodyMedium, styles.empty]}>No chats yet. Ask something above to start one.</Text>
          ) : null}
          {recent?.length ? (
            <View style={styles.group}>
              {recent.map((s, i) => (
                <View key={s.id}>
                  {i > 0 ? <View style={styles.divider} /> : null}
                  <Pressable
                    onPress={() => open({ pathname: '/session/[id]', params: { id: s.id, title: s.title } })}
                    style={({ hovered, pressed }: { pressed: boolean; hovered?: boolean }) => [
                      styles.item,
                      (hovered || pressed) && styles.hover,
                    ]}
                    accessibilityRole="button"
                    accessibilityLabel={s.title}
                  >
                    <Icon
                      name={s.project_id === CHATS_ID ? 'chat-outline' : 'folder-outline'}
                      size={20}
                      color={m3.onSurfaceVariant}
                    />
                    <View style={styles.grow}>
                      <Text style={type.bodyLarge} numberOfLines={1}>
                        {s.title}
                      </Text>
                      <Text style={type.bodySmall} numberOfLines={1}>
                        {s.project_id === CHATS_ID ? 'Chat' : s.project_name} · {relativeTime(s.updated_at)}
                      </Text>
                    </View>
                    {s.status === 'running' || s.status === 'error' ? <StatusBadge status={s.status} /> : null}
                  </Pressable>
                </View>
              ))}
            </View>
          ) : null}

          <View style={styles.sectionRow}>
            <Text style={[type.titleSmall, styles.section]}>Projects</Text>
            <Pressable onPress={() => setAdding(true)} hitSlop={8} accessibilityRole="button">
              <Text style={[type.labelLarge, styles.link]}>New project</Text>
            </Pressable>
          </View>
          {projects?.length === 0 ? (
            <Text style={[type.bodyMedium, styles.empty]}>
              A project is a folder on your PC the agent can work in. Give one a name and Relay makes the folder.
            </Text>
          ) : null}
          {projects?.length ? (
            <View style={styles.group}>
              {projects.map((p, i) => (
                <View key={p.id}>
                  {i > 0 ? <View style={styles.divider} /> : null}
                  <View style={styles.projectRow}>
                    <Pressable
                      onPress={() => open({ pathname: '/project/[id]', params: { id: p.id, name: p.name } })}
                      onLongPress={() => confirmDelete(p)}
                      style={({ hovered, pressed }: { pressed: boolean; hovered?: boolean }) => [
                        styles.item,
                        styles.grow,
                        (hovered || pressed) && styles.hover,
                      ]}
                      accessibilityRole="button"
                      accessibilityLabel={p.name}
                      accessibilityHint="Opens its chats. Long press to remove."
                    >
                      <Avatar label={p.name} size={32} />
                      <Text style={[type.bodyLarge, styles.grow]} numberOfLines={1}>
                        {p.name}
                      </Text>
                    </Pressable>
                    <IconButton icon="dots-vertical" label={`Remove ${p.name}`} onPress={() => confirmDelete(p)} />
                  </View>
                </View>
              ))}
            </View>
          ) : null}

          {wide ? <PairCard /> : null}
        </View>
      </ScrollView>

      <AddProjectSheet
        visible={adding}
        onClose={() => setAdding(false)}
        onCreate={async (name, path) => {
          const p = await client.createProject(name, path);
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
    minHeight: 88,
    maxHeight: 320,
    color: m3.onSurface,
    fontSize: 17,
    lineHeight: 25,
    paddingHorizontal: space.md,
    paddingTop: space.sm,
  },
  inputPhone: { minHeight: 56, fontSize: 16, lineHeight: 22 },
  row: { flexDirection: 'row', alignItems: 'center', gap: space.sm, marginTop: space.sm },
  chip: {
    flexDirection: 'row',
    alignItems: 'center',
    gap: 6,
    height: 36,
    maxWidth: 300,
    paddingHorizontal: space.md,
    borderRadius: radius.sm,
    borderWidth: 1,
    borderColor: m3.outlineVariant,
  },
  hover: { backgroundColor: m3.hover },
  send: { width: 44, height: 44, borderRadius: 22, alignItems: 'center', justifyContent: 'center' },
  sendOn: { backgroundColor: m3.primary },
  sendOff: { backgroundColor: m3.surfaceContainerHighest },
  menu: {
    position: 'absolute',
    top: '100%',
    left: space.md,
    marginTop: space.xs,
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
