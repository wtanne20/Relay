import { Redirect, router } from 'expo-router';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { ActivityIndicator, Pressable, ScrollView, StyleSheet, Text, TextInput, View } from 'react-native';
import { useSafeAreaInsets } from 'react-native-safe-area-context';

import { AttachmentStrip, useAttachments } from '../components/Attachments';
import { m3, radius, space, type } from '../components/theme';
import { Banner, Button, Icon, type IconName } from '../components/ui';
import { describeError } from '../lib/api';
import { useConnection } from '../lib/connection';
import { emit, on } from '../lib/events';
import { formatBytes, relativeTime } from '../lib/format';
import { usePageStyle } from '../lib/layout';
import { clearShare, isLink, pendingShare, sharedText, type SharedPayload } from '../lib/share';
import { CHATS_ID, type Project, type RecentSession } from '../lib/types';

/** Where a share goes: a new chat (plain or in a project) or a recent chat. */
type Dest = { kind: 'new'; projectId: string } | { kind: 'chat'; sessionId: string };

const RECENT_SHOWN = 5;

/** One-tap asks for what was shared; tapping one again takes it back off. */
const QUICK: { label: string; prompt: string; icon: IconName }[] = [
  { label: 'Summarize', prompt: 'Summarize this.', icon: 'text-short' },
  { label: 'Explain', prompt: 'Explain this simply.', icon: 'lightbulb-outline' },
  { label: 'Key points', prompt: 'List the key points and anything I need to act on.', icon: 'format-list-bulleted' },
];

/**
 * What another app shared to Relay (a link, text, screenshots, a voice note
 * or other files), ready to send: add a note or a quick ask, pick a new or
 * recent chat, and send.
 */
export default function ShareScreen() {
  const { loading, client } = useConnection();
  const [payload, setPayload] = useState<SharedPayload | null>(pendingShare);
  useEffect(() => on('share', () => setPayload(pendingShare())), []);

  if (loading) {
    return (
      <View style={styles.center}>
        <ActivityIndicator color={m3.primary} />
      </View>
    );
  }
  if (!client) return <Redirect href="/connect" />;
  if (!payload) {
    return (
      <View style={styles.center}>
        <Icon name="share-variant-outline" size={40} color={m3.outline} />
        <Text style={[type.bodyLarge, styles.muted]}>Nothing to send</Text>
        <Text style={[type.bodyMedium, styles.muted, styles.centerText]}>
          In another app, tap Share and pick Relay to send a link, text, a screenshot or a voice note here.
        </Text>
      </View>
    );
  }
  return <ShareForm key={payload.id} payload={payload} />;
}

function ShareForm({ payload }: { payload: SharedPayload }) {
  const client = useConnection().client!;
  const page = usePageStyle();
  const insets = useSafeAreaInsets();
  const att = useAttachments();
  const { addLocal } = att;
  const shared = useMemo(() => sharedText(payload), [payload]);
  const [text, setText] = useState(shared);
  const [quick, setQuick] = useState<string | null>(null);
  const [projects, setProjects] = useState<Project[] | null>(null);
  const [recent, setRecent] = useState<RecentSession[]>([]);
  const [dest, setDest] = useState<Dest>({ kind: 'new', projectId: CHATS_ID });
  const [reading, setReading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const input = useRef<React.ComponentRef<typeof TextInput>>(null);

  const readFor = useRef<number | null>(null);
  useEffect(() => {
    // Once per share, even if the effect runs twice.
    if (readFor.current === payload.id) return;
    readFor.current = payload.id;
    let alive = true;
    const ready = payload.files.filter((f) => f.uri);
    addLocal(ready.map((f) => ({ uri: f.uri!, name: f.name, mime: f.mime, size: f.size }))).finally(() => {
      if (alive) setReading(false);
    });
    return () => {
      alive = false;
    };
  }, [payload, addLocal]);

  const load = useCallback(async () => {
    try {
      const [ps, rs] = await Promise.all([client.projects(), client.recent(RECENT_SHOWN + 3)]);
      setProjects(ps);
      setRecent(rs.filter((s) => !s.archived).slice(0, RECENT_SHOWN));
      setError(null);
    } catch (e) {
      setError(describeError(e));
    }
  }, [client]);
  useEffect(() => {
    load();
  }, [load]);

  const skipped = payload.files.filter((f) => !f.uri);
  const media = payload.files.filter((f) => f.uri && /^(audio|video)\//.test(f.mime));
  const prompt = [quick, text.trim()].filter(Boolean).join('\n\n');
  const canSend = (!!prompt || att.items.length > 0) && !busy && !reading;

  const send = async () => {
    if (!canSend) return;
    setBusy(true);
    setError(null);
    try {
      let id: string;
      let title: string | undefined;
      if (dest.kind === 'new') {
        const s = await client.createSession(dest.projectId);
        id = s.id;
        title = s.title;
      } else {
        id = dest.sessionId;
        title = recent.find((s) => s.id === dest.sessionId)?.title;
      }
      await client.sendPrompt(id, prompt, att.images, att.files);
      clearShare();
      emit('chats');
      router.replace({ pathname: '/session/[id]', params: { id, title } });
    } catch (e) {
      setError(describeError(e));
      setBusy(false);
    }
  };

  const option = (
    key: string,
    on: boolean,
    icon: IconName,
    label: string,
    sub: string,
    pick: () => void,
    off?: boolean,
  ) => (
    <Pressable
      key={key}
      onPress={pick}
      disabled={off}
      style={({ hovered, pressed }: { pressed: boolean; hovered?: boolean }) => [
        styles.item,
        (hovered || pressed) && styles.hover,
        off && styles.disabled,
      ]}
      accessibilityRole="radio"
      accessibilityState={{ checked: on, disabled: !!off }}
      accessibilityLabel={sub ? `${label}, ${sub}` : label}
    >
      <Icon name={on ? 'radiobox-marked' : 'radiobox-blank'} size={22} color={on ? m3.primary : m3.outline} />
      <Icon name={icon} size={20} color={m3.onSurfaceVariant} />
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
  const isNew = (projectId: string) => dest.kind === 'new' && dest.projectId === projectId;
  const isChat = (sessionId: string) => dest.kind === 'chat' && dest.sessionId === sessionId;
  const destName =
    dest.kind === 'chat'
      ? 'this chat'
      : dest.projectId === CHATS_ID
        ? 'a new chat'
        : `a new chat in ${projects?.find((p) => p.id === dest.projectId)?.name ?? 'the project'}`;

  return (
    <View style={styles.flex}>
      <ScrollView
        style={styles.flex}
        contentContainerStyle={[styles.content, page, { paddingBottom: space.xl }]}
        keyboardShouldPersistTaps="handled"
      >
        <View style={styles.composer}>
          <AttachmentStrip items={att.items} onRemove={att.remove} />
          {reading && payload.files.some((f) => f.uri) ? (
            <View style={styles.reading}>
              <ActivityIndicator size="small" color={m3.primary} />
              <Text style={type.bodySmall}>Reading what you shared…</Text>
            </View>
          ) : null}
          <TextInput
            ref={input}
            style={styles.input}
            value={text}
            onChangeText={setText}
            placeholder={shared ? 'What should Relay do with this?' : 'Add a note (optional)'}
            placeholderTextColor={m3.outline}
            selectionColor={m3.primary}
            multiline
            accessibilityLabel="Message"
          />
          {shared && isLink(shared) && text.trim() === shared ? (
            <Text style={[type.bodySmall, styles.hint]}>
              The model sees the link as text. Ask it to fetch the page if it has a tool for that.
            </Text>
          ) : null}
        </View>

        <View style={styles.chips}>
          {QUICK.map((q) => {
            const onIt = quick === q.prompt;
            return (
              <Pressable
                key={q.label}
                onPress={() => setQuick(onIt ? null : q.prompt)}
                style={[styles.chip, onIt && styles.chipOn]}
                accessibilityRole="button"
                accessibilityState={{ selected: onIt }}
                accessibilityLabel={q.label}
              >
                <Icon
                  name={onIt ? 'check' : q.icon}
                  size={16}
                  color={onIt ? m3.onSecondaryContainer : m3.onSurfaceVariant}
                />
                <Text style={[type.labelLarge, { color: onIt ? m3.onSecondaryContainer : m3.onSurfaceVariant }]}>
                  {q.label}
                </Text>
              </Pressable>
            );
          })}
        </View>

        {skipped.length || payload.dropped ? (
          <Banner
            kind="warning"
            message={[
              ...skipped.map((f) =>
                f.error === 'too_big'
                  ? `${f.name} (${formatBytes(f.size)}) is larger than 25 MB, so it was left out.`
                  : `Relay couldn't read ${f.name}, so it was left out.`,
              ),
              payload.dropped
                ? `${payload.dropped} more file${payload.dropped === 1 ? ' was' : 's were'} left out.`
                : '',
            ]
              .filter(Boolean)
              .join(' ')}
          />
        ) : null}
        {media.length ? (
          <Banner
            kind="info"
            message="The model can't listen to audio or watch video yet. It gets the file's name and size, so say what it's about in your note."
          />
        ) : null}
        {att.error ? <Banner message={att.error} /> : null}
        {error ? <Banner message={error} actionLabel="Retry" onAction={load} /> : null}

        <Text style={[type.titleSmall, styles.section]}>Start a new chat</Text>
        <View style={styles.group}>
          {option(`n:${CHATS_ID}`, isNew(CHATS_ID), 'chat-outline', 'Chat', 'Ask anything. No files or code', () =>
            setDest({ kind: 'new', projectId: CHATS_ID }),
          )}
          {projects === null ? <ActivityIndicator style={styles.spinner} color={m3.primary} /> : null}
          {projects?.map((p) =>
            option(`n:${p.id}`, isNew(p.id), 'folder-outline', p.name, 'New chat in this project', () =>
              setDest({ kind: 'new', projectId: p.id }),
            ),
          )}
        </View>

        {recent.length ? (
          <>
            <Text style={[type.titleSmall, styles.section]}>Or add to a recent chat</Text>
            <View style={styles.group}>
              {recent.map((s) => {
                const running = s.status === 'running';
                const where = s.project_id === CHATS_ID ? '' : `${s.project_name} · `;
                return option(
                  `c:${s.id}`,
                  isChat(s.id),
                  'message-reply-text-outline',
                  s.title,
                  running ? `${where}Working, try again when it's done` : `${where}${relativeTime(s.updated_at)}`,
                  () => setDest({ kind: 'chat', sessionId: s.id }),
                  running,
                );
              })}
            </View>
          </>
        ) : null}
      </ScrollView>

      <View style={[styles.footer, { paddingBottom: insets.bottom + space.md }]}>
        <Button
          title={`Send to ${destName}`}
          icon="send"
          onPress={send}
          disabled={!canSend}
          loading={busy}
          style={styles.sendButton}
        />
      </View>
    </View>
  );
}

const styles = StyleSheet.create({
  flex: { flex: 1, backgroundColor: m3.surface },
  center: {
    flex: 1,
    alignItems: 'center',
    justifyContent: 'center',
    gap: space.sm,
    padding: space.xxl,
    backgroundColor: m3.surface,
  },
  centerText: { textAlign: 'center' },
  muted: { color: m3.onSurfaceVariant },
  content: { padding: space.lg, gap: space.md },
  composer: {
    backgroundColor: m3.surfaceContainerHigh,
    borderRadius: radius.xl,
    padding: space.sm,
    borderWidth: 1,
    borderColor: m3.outlineVariant,
  },
  reading: {
    flexDirection: 'row',
    alignItems: 'center',
    gap: space.sm,
    paddingHorizontal: space.md,
    paddingTop: space.md,
  },
  input: {
    minHeight: 88,
    maxHeight: 280,
    color: m3.onSurface,
    fontSize: 16,
    lineHeight: 22,
    paddingHorizontal: space.md,
    paddingVertical: space.sm,
    textAlignVertical: 'top',
  },
  hint: { paddingHorizontal: space.md, paddingBottom: space.sm, color: m3.outline },
  chips: { flexDirection: 'row', flexWrap: 'wrap', gap: space.sm },
  chip: {
    flexDirection: 'row',
    alignItems: 'center',
    gap: 6,
    height: 36,
    paddingHorizontal: space.md,
    borderRadius: radius.sm,
    borderWidth: 1,
    borderColor: m3.outlineVariant,
  },
  chipOn: { backgroundColor: m3.secondaryContainer, borderColor: m3.secondaryContainer },
  section: { color: m3.onSurfaceVariant, marginTop: space.sm, marginLeft: space.xs },
  group: { backgroundColor: m3.surfaceContainerLow, borderRadius: radius.lg, overflow: 'hidden' },
  item: {
    flexDirection: 'row',
    alignItems: 'center',
    gap: space.md,
    minHeight: 56,
    paddingHorizontal: space.lg,
    paddingVertical: space.sm,
  },
  grow: { flex: 1, minWidth: 0 },
  hover: { backgroundColor: m3.hover },
  disabled: { opacity: 0.5 },
  spinner: { margin: space.md },
  footer: {
    paddingHorizontal: space.lg,
    paddingTop: space.md,
    borderTopWidth: 1,
    borderTopColor: m3.outlineVariant,
    backgroundColor: m3.surface,
  },
  sendButton: { alignSelf: 'stretch' },
});
