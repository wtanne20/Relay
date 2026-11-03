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

import { AssistantBlock, DraftBlock, UserBubble } from '../../components/ChatItems';
import { colors, radius, space, touch } from '../../components/theme';
import { Banner, EmptyState, StatusBadge } from '../../components/ui';
import { ApiError, describeError } from '../../lib/api';
import { useClient } from '../../lib/connection';
import type { Message } from '../../lib/types';
import { useSessionStream } from '../../lib/useSessionStream';

type Item =
  | { key: string; kind: 'user'; message: Message }
  | { key: string; kind: 'assistant'; message: Message }
  | { key: string; kind: 'draft'; content: string; reasoning: string };

export default function ChatScreen() {
  const client = useClient();
  const { id, title: initialTitle } = useLocalSearchParams<{ id: string; title?: string }>();
  const insets = useSafeAreaInsets();
  const stream = useSessionStream(client, id);
  const [title, setTitle] = useState(initialTitle ?? 'Session');
  const [input, setInput] = useState('');
  const [sending, setSending] = useState(false);
  const [stopping, setStopping] = useState(false);
  const [sendError, setSendError] = useState<string | null>(null);

  useEffect(() => {
    if (!client) {
      router.replace('/connect');
      return;
    }
    client
      .session(id)
      .then((s) => setTitle(s.title))
      .catch(() => undefined);
  }, [client, id]);

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
    const out: Item[] = [];
    for (const m of stream.messages) {
      if (m.role === 'user') out.push({ key: `m${m.id}`, kind: 'user', message: m });
      else if (m.role === 'assistant') out.push({ key: `m${m.id}`, kind: 'assistant', message: m });
    }
    if (running && stream.draft) {
      out.push({ key: 'draft', kind: 'draft', content: stream.draft.content, reasoning: stream.draft.reasoning });
    }
    return out.reverse();
  }, [stream.messages, stream.draft, running]);

  // Only the newest assistant message can have tool calls still running.
  const lastAssistantId = useMemo(() => {
    for (let i = stream.messages.length - 1; i >= 0; i--) {
      if (stream.messages[i].role === 'assistant') return stream.messages[i].id;
    }
    return -1;
  }, [stream.messages]);

  if (!client) return null;

  const send = async () => {
    const text = input.trim();
    if (!text || sending) return;
    setSending(true);
    setSendError(null);
    try {
      const res = await client.sendPrompt(id, text);
      stream.applyMessages([res.message]);
      stream.markRunning();
      setInput('');
      if (stream.link !== 'live') stream.reconnect();
    } catch (e) {
      if (e instanceof ApiError && e.status === 409) {
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

  const linkLabel =
    stream.link === 'live' ? null : stream.link === 'connecting' ? 'Connecting…' : 'Offline · retrying';

  return (
    <View style={[styles.flex, { paddingTop: insets.top }]}>
      <View style={styles.header}>
        <Pressable
          onPress={() => (router.canGoBack() ? router.back() : router.replace('/'))}
          style={styles.back}
          hitSlop={8}
          accessibilityRole="button"
          accessibilityLabel="Back"
        >
          <Text style={styles.backText}>‹</Text>
        </Pressable>
        <View style={styles.headerText}>
          <Text style={styles.title} numberOfLines={1}>
            {title}
          </Text>
          {linkLabel ? (
            <Pressable onPress={stream.reconnect} hitSlop={6} accessibilityRole="button">
              <Text style={styles.link}>{linkLabel}</Text>
            </Pressable>
          ) : null}
        </View>
        <StatusBadge status={stream.status} />
      </View>

      <KeyboardAvoidingView style={styles.flex} behavior="padding">
        <FlatList
          inverted
          data={items}
          keyExtractor={(it) => it.key}
          contentContainerStyle={styles.list}
          keyboardDismissMode="interactive"
          keyboardShouldPersistTaps="handled"
          onEndReached={stream.loadOlder}
          onEndReachedThreshold={0.3}
          ListFooterComponent={
            stream.loadingOlder ? <ActivityIndicator style={{ margin: space.md }} color={colors.textMuted} /> : undefined
          }
          ListEmptyComponent={
            stream.initialLoaded ? (
              <View style={styles.emptyWrap}>
                <EmptyState title="Ask the agent something" body="It works inside this project's folder on your PC." />
              </View>
            ) : (
              <ActivityIndicator style={{ margin: space.xl }} color={colors.accent} />
            )
          }
          renderItem={({ item }) => {
            if (item.kind === 'user') return <UserBubble message={item.message} />;
            if (item.kind === 'draft') return <DraftBlock content={item.content} reasoning={item.reasoning} />;
            return (
              <AssistantBlock
                message={item.message}
                results={results}
                running={running && item.message.id === lastAssistantId}
              />
            );
          }}
        />

        {stream.linkError && stream.link !== 'live' ? (
          <Banner kind="warning" message={stream.linkError} actionLabel="Retry" onAction={stream.reconnect} />
        ) : null}
        {stream.status === 'error' && stream.taskError ? <Banner message={stream.taskError} /> : null}
        {sendError ? <Banner message={sendError} actionLabel="Dismiss" onAction={() => setSendError(null)} /> : null}

        <View style={[styles.composer, { paddingBottom: Math.max(insets.bottom, space.sm) }]}>
          <TextInput
            style={styles.input}
            value={input}
            onChangeText={setInput}
            placeholder={running ? 'Agent is working…' : 'Message the agent'}
            placeholderTextColor={colors.textFaint}
            multiline
            autoCapitalize="sentences"
            accessibilityLabel="Prompt"
          />
          {running ? (
            <Pressable
              onPress={stop}
              disabled={stopping}
              style={({ pressed }) => [styles.action, styles.stop, (pressed || stopping) && { opacity: 0.6 }]}
              accessibilityRole="button"
              accessibilityLabel="Stop the agent"
            >
              {stopping ? <ActivityIndicator color={colors.danger} /> : <Text style={styles.stopText}>■</Text>}
            </Pressable>
          ) : (
            <Pressable
              onPress={send}
              disabled={!input.trim() || sending}
              style={({ pressed }) => [
                styles.action,
                styles.send,
                (!input.trim() || sending) && { opacity: 0.4 },
                pressed && { backgroundColor: colors.accentPressed },
              ]}
              accessibilityRole="button"
              accessibilityLabel="Send"
            >
              {sending ? <ActivityIndicator color={colors.onAccent} /> : <Text style={styles.sendText}>↑</Text>}
            </Pressable>
          )}
        </View>
      </KeyboardAvoidingView>
    </View>
  );
}

const styles = StyleSheet.create({
  flex: { flex: 1, backgroundColor: colors.bg },
  header: {
    minHeight: 56,
    flexDirection: 'row',
    alignItems: 'center',
    paddingRight: space.lg,
    backgroundColor: colors.surface,
    borderBottomWidth: StyleSheet.hairlineWidth,
    borderBottomColor: colors.border,
    gap: space.sm,
  },
  back: { width: touch, height: touch, alignItems: 'center', justifyContent: 'center' },
  backText: { color: colors.text, fontSize: 34, lineHeight: 36 },
  headerText: { flex: 1 },
  title: { color: colors.text, fontSize: 17, fontWeight: '600' },
  link: { color: colors.warning, fontSize: 12, marginTop: 1 },
  list: { paddingHorizontal: space.lg, paddingVertical: space.md },
  emptyWrap: { transform: [{ scaleY: -1 }] },
  composer: {
    flexDirection: 'row',
    alignItems: 'flex-end',
    gap: space.sm,
    paddingHorizontal: space.md,
    paddingTop: space.sm,
    backgroundColor: colors.surface,
    borderTopWidth: StyleSheet.hairlineWidth,
    borderTopColor: colors.border,
  },
  input: {
    flex: 1,
    minHeight: touch,
    maxHeight: 160,
    backgroundColor: colors.bg,
    borderRadius: radius.lg,
    borderWidth: StyleSheet.hairlineWidth,
    borderColor: colors.border,
    color: colors.text,
    fontSize: 16,
    paddingHorizontal: space.md,
    paddingTop: 13,
    paddingBottom: 13,
  },
  action: { width: touch, height: touch, borderRadius: touch / 2, alignItems: 'center', justifyContent: 'center' },
  send: { backgroundColor: colors.accent },
  sendText: { color: colors.onAccent, fontSize: 22, fontWeight: '800' },
  stop: { backgroundColor: colors.dangerBg, borderWidth: 1, borderColor: colors.danger },
  stopText: { color: colors.danger, fontSize: 18 },
});
