import { router } from 'expo-router';
import { useCallback, useEffect, useRef, useState } from 'react';
import { ActivityIndicator, Image, Pressable, ScrollView, StyleSheet, Text, TextInput, View } from 'react-native';

import { describeError } from '../lib/api';
import { useConnection } from '../lib/connection';
import { emit, on } from '../lib/events';
import { enterToSend } from '../lib/layout';
import type { Project } from '../lib/types';
import { AddProjectSheet } from './AddProjectSheet';
import { m3, radius, space, type } from './theme';
import { Banner, Button, Icon, Logo } from './ui';

const LAST_PROJECT = 'relay.lastProject';

function greeting(): string {
  const h = new Date().getHours();
  if (h < 5) return 'Up late?';
  if (h < 12) return 'Good morning';
  if (h < 18) return 'Good afternoon';
  return 'Good evening';
}

/** A chat title from the first line of the prompt, like Claude's app. */
function titleFrom(text: string): string {
  const line = text.split('\n')[0].trim();
  return line.length > 60 ? `${line.slice(0, 57).trimEnd()}…` : line;
}

function remembered(): string | null {
  try {
    return globalThis.localStorage?.getItem(LAST_PROJECT) ?? null;
  } catch {
    return null;
  }
}

function remember(id: string) {
  try {
    globalThis.localStorage?.setItem(LAST_PROJECT, id);
  } catch {
    // Not important: the first project is picked next time.
  }
}

/** Desktop start page: greeting plus a composer that starts a new chat. */
export function DesktopHome() {
  const { client } = useConnection();
  const [projects, setProjects] = useState<Project[] | null>(null);
  const [projectId, setProjectId] = useState<string | null>(remembered());
  const [picking, setPicking] = useState(false);
  const [adding, setAdding] = useState(false);
  const [text, setText] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const input = useRef<React.ComponentRef<typeof TextInput>>(null);

  const load = useCallback(async () => {
    if (!client) return;
    try {
      const ps = await client.projects();
      setProjects(ps);
      setProjectId((cur) => (cur && ps.some((p) => p.id === cur) ? cur : (ps[0]?.id ?? null)));
      setError(null);
    } catch (e) {
      setError(describeError(e));
    }
  }, [client]);

  useEffect(() => {
    load();
    return on('projects', load);
  }, [load]);

  const project = projects?.find((p) => p.id === projectId) ?? null;
  const canSend = !!text.trim() && !!project && !busy;

  const send = async () => {
    if (!client || !project || !text.trim() || busy) return;
    setBusy(true);
    setError(null);
    try {
      const prompt = text.trim();
      const s = await client.createSession(project.id, titleFrom(prompt));
      await client.sendPrompt(s.id, prompt);
      setText('');
      emit('chats');
      router.replace({ pathname: '/session/[id]', params: { id: s.id, title: s.title } });
    } catch (e) {
      setError(describeError(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <ScrollView style={styles.flex} contentContainerStyle={styles.content} keyboardShouldPersistTaps="handled">
      <View style={styles.column}>
        <View style={styles.hello}>
          <Logo size={44} />
          <Text style={[type.displaySmall, styles.greeting]}>{greeting()}</Text>
        </View>

        {projects !== null && projects.length === 0 ? (
          <View style={styles.firstRun}>
            <Text style={[type.titleLarge, styles.center]}>Add your first project</Text>
            <Text style={[type.bodyLarge, styles.muted]}>
              Give it a name and Relay makes its folder. The agent reads, edits and runs code only inside it.
            </Text>
            <Button title="Add project" icon="folder-plus-outline" onPress={() => setAdding(true)} />
          </View>
        ) : (
          <View style={styles.composer}>
            <TextInput
              ref={input}
              style={styles.input}
              value={text}
              onChangeText={setText}
              onKeyPress={enterToSend(send)}
              placeholder="How can the agent help today?"
              placeholderTextColor={m3.outline}
              selectionColor={m3.primary}
              multiline
              autoFocus
              accessibilityLabel="Prompt"
            />
            <View style={styles.row}>
              <Pressable
                onPress={() => setPicking((v) => !v)}
                style={({ hovered }: { pressed: boolean; hovered?: boolean }) => [
                  styles.chip,
                  hovered && styles.chipHover,
                ]}
                accessibilityRole="button"
                accessibilityLabel={`Project: ${project?.name ?? 'none'}. Change project`}
              >
                <Icon name="folder-outline" size={18} color={m3.onSurfaceVariant} />
                <Text style={[type.labelLarge, { color: m3.onSurfaceVariant }]} numberOfLines={1}>
                  {project?.name ?? 'Choose a project'}
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
              <View style={styles.menu}>
                {projects?.map((p) => (
                  <Pressable
                    key={p.id}
                    onPress={() => {
                      setProjectId(p.id);
                      remember(p.id);
                      setPicking(false);
                      input.current?.focus();
                    }}
                    style={({ hovered }: { pressed: boolean; hovered?: boolean }) => [
                      styles.menuItem,
                      hovered && styles.chipHover,
                    ]}
                    accessibilityRole="menuitem"
                  >
                    <Icon
                      name={p.id === projectId ? 'check' : 'folder-outline'}
                      size={18}
                      color={p.id === projectId ? m3.primary : m3.onSurfaceVariant}
                    />
                    <View style={styles.grow}>
                      <Text style={type.bodyLarge} numberOfLines={1}>
                        {p.name}
                      </Text>
                      <Text style={type.bodySmall} numberOfLines={1}>
                        {p.path}
                      </Text>
                    </View>
                  </Pressable>
                ))}
                <Pressable
                  onPress={() => {
                    setPicking(false);
                    setAdding(true);
                  }}
                  style={({ hovered }: { pressed: boolean; hovered?: boolean }) => [
                    styles.menuItem,
                    hovered && styles.chipHover,
                  ]}
                  accessibilityRole="menuitem"
                >
                  <Icon name="folder-plus-outline" size={18} color={m3.primary} />
                  <Text style={[type.labelLarge, { color: m3.primary }]}>Add project…</Text>
                </Pressable>
              </View>
            ) : null}
          </View>
        )}

        {error ? <Banner message={error} actionLabel="Retry" onAction={load} /> : null}
        {project ? (
          <Text style={[type.bodySmall, styles.hint]}>
            Works in {project.path} · Enter to send · Shift+Enter for a new line
          </Text>
        ) : null}
        <PairCard />
      </View>

      <AddProjectSheet
        visible={adding}
        onClose={() => setAdding(false)}
        onCreate={async (name, path) => {
          if (!client) return;
          const p = await client.createProject(name, path);
          setAdding(false);
          setProjectId(p.id);
          remember(p.id);
          emit('projects');
        }}
      />
    </ScrollView>
  );
}

/** Small pairing QR on the home screen; opens Settings for the larger code and details. */
function PairCard() {
  const { client } = useConnection();
  const [svg, setSvg] = useState<string | null>(null);
  useEffect(() => {
    client
      ?.pairing()
      .then((p) => setSvg(p.svg))
      .catch(() => setSvg(null));
  }, [client]);
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
  grow: { flex: 1, minWidth: 0 },
  center: { textAlign: 'center' },
  muted: { color: m3.onSurfaceVariant, textAlign: 'center' },
  content: { flexGrow: 1, justifyContent: 'center', padding: space.xl, paddingBottom: '12%' },
  column: { width: '100%', maxWidth: 720, alignSelf: 'center', gap: space.lg },
  hello: {
    flexDirection: 'row',
    alignItems: 'center',
    justifyContent: 'center',
    gap: space.lg,
    marginBottom: space.lg,
  },
  greeting: { fontSize: 40, lineHeight: 48 },
  firstRun: {
    alignItems: 'center',
    gap: space.md,
    backgroundColor: m3.surfaceContainerLow,
    borderRadius: radius.xl,
    padding: space.xxl,
  },
  composer: {
    backgroundColor: m3.surfaceContainerHigh,
    borderRadius: radius.xl,
    padding: space.md,
    zIndex: 10,
    borderWidth: 1,
    borderColor: m3.outlineVariant,
  },
  input: {
    minHeight: 96,
    maxHeight: 320,
    color: m3.onSurface,
    fontSize: 17,
    lineHeight: 25,
    paddingHorizontal: space.md,
    paddingTop: space.sm,
  },
  row: { flexDirection: 'row', alignItems: 'center', gap: space.sm, marginTop: space.sm },
  chip: {
    flexDirection: 'row',
    alignItems: 'center',
    gap: 6,
    height: 36,
    maxWidth: 360,
    paddingHorizontal: space.md,
    borderRadius: radius.sm,
    borderWidth: 1,
    borderColor: m3.outlineVariant,
  },
  chipHover: { backgroundColor: m3.hover },
  send: { width: 44, height: 44, borderRadius: 22, alignItems: 'center', justifyContent: 'center' },
  sendOn: { backgroundColor: m3.primary },
  sendOff: { backgroundColor: m3.surfaceContainerHighest },
  menu: {
    position: 'absolute',
    top: '100%',
    left: space.md,
    marginTop: space.xs,
    width: 380,
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
  menuItem: {
    flexDirection: 'row',
    alignItems: 'center',
    gap: space.md,
    minHeight: 48,
    paddingHorizontal: space.lg,
    paddingVertical: space.xs,
  },
  hint: { textAlign: 'center', color: m3.outline },
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
});
