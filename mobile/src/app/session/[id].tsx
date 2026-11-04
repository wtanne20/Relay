import { router, useLocalSearchParams } from 'expo-router';
import { useEffect, useMemo, useState } from 'react';
import {
  ActivityIndicator,
  FlatList,
  KeyboardAvoidingView,
  Pressable,
  StyleSheet,
  Text,
  TextInput,
  View,
} from 'react-native';
import { useSafeAreaInsets } from 'react-native-safe-area-context';

import { AssistantTurn, CompactingRow, noteStart, SummaryDivider, UserBubble } from '../../components/ChatItems';
import { ApprovalCard } from '../../components/ApprovalCard';
import { AttachButtons, AttachmentStrip, useAttachments } from '../../components/Attachments';
import { ModelChip, ModelSheet, modelLabel } from '../../components/ModelPicker';
import { m3, radius, space, type } from '../../components/theme';
import { Banner, Icon, IconButton, StatusBadge } from '../../components/ui';
import { ApiError, describeError } from '../../lib/api';
import { useClient } from '../../lib/connection';
import { enterToSend, usePageStyle, useWide } from '../../lib/layout';
import { emit } from '../../lib/events';
import { CHATS_ID, type Message, type Project, type Session } from '../../lib/types';
import { useSessionStream } from '../../lib/useSessionStream';

type Item =
  | { key: string; kind: 'user'; message: Message }
  | { key: string; kind: 'turn'; messages: Message[]; latest: boolean }
  | { key: string; kind: 'summary'; message: Message }
  | { key: string; kind: 'compacting' };

export default function ChatScreen() {
  const client = useClient();
  const { id, title: initialTitle } = useLocalSearchParams<{ id: string; title?: string }>();
  const insets = useSafeAreaInsets();
  const stream = useSessionStream(client, id);
  const wide = useWide();
  const page = usePageStyle();
  const [title, setTitle] = useState(initialTitle ?? 'Session');
  const [input, setInput] = useState('');
  const [sending, setSending] = useState(false);
  const [stopping, setStopping] = useState(false);
  const [sendError, setSendError] = useState<string | null>(null);
  const pics = useAttachments();
  const [session, setSession] = useState<Session | null>(null);
  const [project, setProject] = useState<Project | null>(null);
  const [loaded, setLoaded] = useState<string | null>(null);
  const [picking, setPicking] = useState(false);

  useEffect(() => {
    if (!client) {
      router.replace('/connect');
      return;
    }
    client
      .session(id)
      .then((s) => {
        setSession(s);
        setTitle(s.title);
        return client.project(s.project_id).then(setProject);
      })
      .catch(() => undefined);
    client
      .model()
      .then((m) => setLoaded(m.model_path || null))
      .catch(() => undefined);
  }, [client, id]);

  // Relay names new chats after the first reply.
  useEffect(() => {
    if (stream.title) {
      setTitle(stream.title);
      emit('chats');
    }
  }, [stream.title]);

  const plain = session?.project_id === CHATS_ID;

  const running = stream.status === 'running';
  useEffect(() => {
    if (!running) setStopping(false);
  }, [running]);

  const results = useMemo(() => {
    const map = new Map<string, Message>();
    for (const m of stream.messages) if (m.role === 'tool' && m.tool_call_id) map.set(m.tool_call_id, m);
    return map;
  }, [stream.messages]);

  // Inverted list: newest first.
  const items = useMemo(() => {
    // Consecutive assistant messages (one agent reply) become one turn.
    const out: Item[] = [];
    stream.messages.forEach((m, i) => {
      if (m.role === 'assistant') noteStart(m, stream.messages[i - 1]);
    });
    for (const m of stream.messages) {
      if (m.role === 'user') out.push({ key: `m${m.id}`, kind: 'user', message: m });
      else if (m.role === 'summary') out.push({ key: `s${m.id}`, kind: 'summary', message: m });
      else if (m.role === 'assistant') {
        const prev = out[out.length - 1];
        if (prev?.kind === 'turn') prev.messages.push(m);
        else out.push({ key: `t${m.id}`, kind: 'turn', messages: [m], latest: false });
      }
    }
    const tail = out[out.length - 1];
    if (tail?.kind === 'turn') tail.latest = true;
    else if (running && !stream.compacting) out.push({ key: 'pending', kind: 'turn', messages: [], latest: true });
    if (running && stream.compacting) out.push({ key: 'compacting', kind: 'compacting' });
    return out.reverse();
  }, [stream.messages, running, stream.compacting]);

  if (!client) return null;

  const send = async () => {
    const text = input.trim();
    if ((!text && !pics.items.length) || sending) return;
    setSending(true);
    setSendError(null);
    try {
      const res = await client.sendPrompt(
        id,
        text,
        pics.items.map((a) => a.dataUrl),
      );
      stream.applyMessages([res.message]);
      stream.markRunning();
      setInput('');
      pics.clear();
      if (stream.link !== 'live') stream.reconnect();
    } catch (e) {
      if (e instanceof ApiError && e.status === 409 && /another chat/i.test(e.message)) {
        setSendError(describeError(e));
      } else if (e instanceof ApiError && e.status === 409) {
        stream.setStatus('running');
        setSendError('The agent is still working. Stop it first or wait for it to finish.');
      } else {
        setSendError(describeError(e));
      }
    } finally {
      setSending(false);
    }
  };

  const stop = async () => {
    setStopping(true);
    try {
      await client.stop(id);
    } catch (e) {
      setSendError(describeError(e));
      setStopping(false);
    }
  };

  const linkLabel = stream.link === 'live' ? null : stream.link === 'connecting' ? 'Connecting…' : 'Offline · retrying';

  const canSend = (!!input.trim() || pics.items.length > 0) && !sending;

  return (
    <View style={[styles.flex, { paddingTop: insets.top }]}>
      <View style={[styles.header, wide && styles.headerWide]}>
        {wide ? null : (
          <IconButton
            icon="arrow-left"
            label="Back"
            color={m3.onSurface}
            onPress={() => (router.canGoBack() ? router.back() : router.replace('/'))}
          />
        )}
        <View style={styles.headerText}>
          <Text style={type.titleMedium} numberOfLines={1}>
            {title}
          </Text>
          {linkLabel ? (
            <Pressable onPress={stream.reconnect} hitSlop={6} accessibilityRole="button" style={styles.linkRow}>
              <Icon name="wifi-off" size={14} color={m3.warning} />
              <Text style={[type.bodySmall, { color: m3.warning }]}>{linkLabel}</Text>
            </Pressable>
          ) : null}
        </View>
        {stream.status !== 'idle' ? <StatusBadge status={stream.status} /> : null}
        <View style={{ width: space.sm }} />
      </View>

      <KeyboardAvoidingView style={styles.flex} behavior="padding">
        {items.length === 0 ? (
          // Rendered outside the inverted list: how platforms flip an inverted
          // list's empty component differs (it came out upside down on web).
          <View style={[styles.flex, styles.emptyOuter]}>
            {stream.initialLoaded ? (
              <View style={styles.emptyWrap}>
                <View style={styles.spark}>
                  <Icon name="creation" size={28} color={m3.primary} />
                </View>
                <Text style={[type.headlineMedium, styles.emptyTitle]}>
                  {plain ? 'What would you like to know?' : 'What should we work on?'}
                </Text>
                <Text style={[type.bodyLarge, styles.emptyBody]}>
                  {plain
                    ? 'A plain chat with your local model. It runs privately on your PC and has no access to your files.'
                    : `The agent reads, edits and runs code in ${project?.name ?? 'this project'} on your PC.`}
                </Text>
              </View>
            ) : (
              <ActivityIndicator style={{ margin: space.xl }} color={m3.primary} />
            )}
          </View>
        ) : (
          <FlatList
            inverted
            data={items}
            extraData={[stream.draft, results, running]}
            keyExtractor={(it) => it.key}
            contentContainerStyle={[styles.list, page]}
            keyboardDismissMode="interactive"
            keyboardShouldPersistTaps="handled"
            onEndReached={stream.loadOlder}
            onEndReachedThreshold={0.3}
            ListFooterComponent={
              stream.loadingOlder ? (
                <ActivityIndicator style={{ margin: space.md }} color={m3.onSurfaceVariant} />
              ) : undefined
            }
            renderItem={({ item }) => {
              if (item.kind === 'user') return <UserBubble message={item.message} />;
              if (item.kind === 'summary') return <SummaryDivider message={item.message} />;
              if (item.kind === 'compacting') return <CompactingRow />;
              return (
                <AssistantTurn
                  messages={item.messages}
                  results={results}
                  running={running && item.latest}
                  draft={item.latest ? stream.draft : null}
                />
              );
            }}
          />
        )}

        <View style={page}>
          {stream.linkError && stream.link !== 'live' ? (
            <Banner kind="warning" message={stream.linkError} actionLabel="Retry" onAction={stream.reconnect} />
          ) : null}
          {stream.status === 'error' && stream.taskError ? <Banner message={stream.taskError} /> : null}
          {sendError ? <Banner message={sendError} actionLabel="Dismiss" onAction={() => setSendError(null)} /> : null}
          {pics.error ? <Banner kind="warning" message={pics.error} actionLabel="Dismiss" onAction={() => pics.setError(null)} /> : null}
          {stream.approval ? (
            <ApprovalCard
              key={stream.approval.tool_call_id}
              approval={stream.approval}
              onDecide={async (decision) => {
                const pending = stream.approval;
                if (!pending) return;
                try {
                  await client.answerApproval(id, pending.tool_call_id, decision);
                  stream.setApproval(null);
                } catch (e) {
                  setSendError(describeError(e));
                  if (e instanceof ApiError && e.status === 409) stream.setApproval(null);
                }
              }}
            />
          ) : null}

          <View style={[styles.composerWrap, { paddingBottom: Math.max(insets.bottom, wide ? space.lg : space.sm) }]}>
            <View style={styles.composer}>
              <AttachmentStrip items={pics.items} onRemove={pics.remove} />
              <TextInput
                style={styles.input}
                value={input}
                onChangeText={setInput}
                placeholder={
                  stream.approval
                    ? 'Waiting for your answer…'
                    : running
                      ? plain
                        ? 'Writing…'
                        : 'Agent is working…'
                      : plain
                        ? 'Message Relay'
                        : 'Message the agent'
                }
                placeholderTextColor={m3.outline}
                selectionColor={m3.primary}
                cursorColor={m3.primary}
                multiline
                autoCapitalize="sentences"
                autoFocus={wide}
                onKeyPress={enterToSend(send)}
                accessibilityLabel="Prompt"
              />
              <View style={styles.composerRow}>
                <View style={styles.composerHint}>
                  <AttachButtons onPick={pics.pick} disabled={sending} />
                  <ModelChip
                    model={session?.model}
                    fallback={project?.model ?? loaded}
                    onPress={() => setPicking(true)}
                    disabled={!session}
                  />
                </View>
                {running ? (
                  <Pressable
                    onPress={stop}
                    disabled={stopping}
                    style={({ pressed }) => [styles.action, styles.stop, (pressed || stopping) && { opacity: 0.7 }]}
                    accessibilityRole="button"
                    accessibilityLabel="Stop the agent"
                  >
                    {stopping ? (
                      <ActivityIndicator color={m3.onErrorContainer} />
                    ) : (
                      <Icon name="stop" size={22} color={m3.onErrorContainer} />
                    )}
                  </Pressable>
                ) : (
                  <Pressable
                    onPress={send}
                    disabled={!canSend}
                    style={({ pressed }) => [
                      styles.action,
                      canSend ? styles.send : styles.sendOff,
                      pressed && { opacity: 0.85 },
                    ]}
                    accessibilityRole="button"
                    accessibilityLabel="Send"
                  >
                    {sending ? (
                      <ActivityIndicator color={m3.onPrimary} />
                    ) : (
                      <Icon name="arrow-up" size={22} color={canSend ? m3.onPrimary : m3.outline} />
                    )}
                  </Pressable>
                )}
              </View>
            </View>
          </View>
        </View>
      </KeyboardAvoidingView>
      <ModelSheet
        visible={picking}
        onClose={() => setPicking(false)}
        value={session?.model}
        defaultTitle={project?.model && !plain ? 'Same as the project' : 'Whichever model is loaded'}
        defaultBody={
          project?.model
            ? `${plain ? 'Your default for chats' : project.name}: ${modelLabel(project.model)}`
            : loaded
              ? `Now: ${modelLabel(loaded)}`
              : 'The model chosen in Settings'
        }
        onPick={async (path) => setSession(await client.setSessionModel(id, path))}
      />
    </View>
  );
}

const styles = StyleSheet.create({
  flex: { flex: 1, backgroundColor: m3.surface },
  header: {
    height: 64,
    flexDirection: 'row',
    alignItems: 'center',
    paddingHorizontal: space.xs,
    gap: space.xs,
  },
  headerWide: {
    paddingLeft: space.xl,
    paddingRight: space.lg,
    borderBottomWidth: 1,
    borderBottomColor: m3.outlineVariant,
  },
  headerText: { flex: 1, minWidth: 0, paddingHorizontal: space.xs },
  linkRow: { flexDirection: 'row', alignItems: 'center', gap: 4 },
  list: { paddingHorizontal: space.lg, paddingVertical: space.md },
  emptyOuter: { justifyContent: 'center' },
  emptyWrap: { alignItems: 'center', paddingHorizontal: space.xl, gap: space.md },
  spark: {
    width: 56,
    height: 56,
    borderRadius: 18,
    backgroundColor: m3.surfaceContainerHigh,
    alignItems: 'center',
    justifyContent: 'center',
  },
  emptyTitle: { textAlign: 'center' },
  emptyBody: { textAlign: 'center', color: m3.onSurfaceVariant },
  composerWrap: { paddingHorizontal: space.md, paddingTop: space.xs },
  composer: {
    backgroundColor: m3.surfaceContainerHigh,
    borderRadius: radius.xl,
    paddingTop: space.sm,
    paddingBottom: space.sm,
    paddingHorizontal: space.sm,
  },
  input: {
    minHeight: 44,
    maxHeight: 180,
    color: m3.onSurface,
    fontSize: 16,
    lineHeight: 22,
    paddingHorizontal: space.md,
    paddingTop: space.sm,
    paddingBottom: space.xs,
  },
  composerRow: { flexDirection: 'row', alignItems: 'center', justifyContent: 'space-between', gap: space.sm },
  composerHint: { flexDirection: 'row', alignItems: 'center', gap: 6, paddingLeft: space.xs, flex: 1, minWidth: 0 },
  action: { width: 44, height: 44, borderRadius: 22, alignItems: 'center', justifyContent: 'center' },
  send: { backgroundColor: m3.primary },
  sendOff: { backgroundColor: m3.surfaceContainerHighest },
  stop: { backgroundColor: m3.errorContainer, borderRadius: 14 },
});
