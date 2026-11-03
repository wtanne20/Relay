import { memo, useState } from 'react';
import { ActivityIndicator, Platform, Pressable, ScrollView, StyleSheet, Text, View } from 'react-native';

import { toolDetails, toolSummary } from '../lib/format';
import type { Message, ToolCall } from '../lib/types';
import { Markdown } from './Markdown';
import { colors, radius, space } from './theme';

const MAX_RESULT_CHARS = 6000;
const monoFont = Platform.OS === 'ios' ? 'Menlo' : 'monospace';

export function UserBubble({ message }: { message: Message }) {
  return (
    <View style={styles.userRow}>
      <View style={styles.userBubble}>
        <Text selectable style={styles.userText}>
          {message.content}
        </Text>
      </View>
    </View>
  );
}

function Reasoning({ text, live }: { text: string; live?: boolean }) {
  const [open, setOpen] = useState(false);
  if (!text.trim()) return null;
  return (
    <View style={styles.reasoning}>
      <Pressable
        onPress={() => setOpen((o) => !o)}
        style={styles.reasoningHeader}
        accessibilityRole="button"
        accessibilityState={{ expanded: open }}
        hitSlop={6}
      >
        <Text style={styles.reasoningLabel}>
          {open ? '▾' : '▸'} {live ? 'Thinking…' : 'Thought process'}
        </Text>
      </Pressable>
      {open ? (
        <Text selectable style={styles.reasoningText}>
          {text.trim()}
        </Text>
      ) : null}
    </View>
  );
}

export const ToolActivityRow = memo(function ToolActivityRow({
  call,
  result,
  running,
}: {
  call: ToolCall;
  result?: Message;
  running: boolean;
}) {
  const [open, setOpen] = useState(false);
  const pending = !result;
  const failed = result?.is_error === true;
  const summary = toolSummary(call, result, running);
  const iconColor = pending ? colors.accent : failed ? colors.danger : colors.success;
  const content = result?.content ?? '';
  const shown = content.length > MAX_RESULT_CHARS ? `${content.slice(0, MAX_RESULT_CHARS)}\n… (truncated)` : content;
  return (
    <View style={styles.tool}>
      <Pressable
        onPress={() => setOpen((o) => !o)}
        style={({ pressed }) => [styles.toolHeader, pressed && { opacity: 0.7 }]}
        accessibilityRole="button"
        accessibilityLabel={summary}
        accessibilityState={{ expanded: open }}
      >
        {pending && running ? (
          <ActivityIndicator size="small" color={iconColor} />
        ) : (
          <Text style={[styles.toolIcon, { color: iconColor }]}>{failed ? '✕' : pending ? '•' : '✓'}</Text>
        )}
        <Text style={[styles.toolSummary, failed && { color: colors.danger }]} numberOfLines={open ? undefined : 1}>
          {summary}
        </Text>
        <Text style={styles.toolChevron}>{open ? '▾' : '▸'}</Text>
      </Pressable>
      {open ? (
        <View style={styles.toolBody}>
          <Text style={styles.toolSection}>{call.name}</Text>
          <ScrollView horizontal nestedScrollEnabled>
            <Text selectable style={styles.toolMono}>
              {toolDetails(call)}
            </Text>
          </ScrollView>
          {result ? (
            <>
              <Text style={styles.toolSection}>Result</Text>
              <ScrollView horizontal nestedScrollEnabled>
                <Text selectable style={styles.toolMono}>
                  {shown || '(empty)'}
                </Text>
              </ScrollView>
            </>
          ) : null}
        </View>
      ) : null}
    </View>
  );
});

export const AssistantBlock = memo(function AssistantBlock({
  message,
  results,
  running,
}: {
  message: Message;
  results: Map<string, Message>;
  running: boolean;
}) {
  const calls = message.tool_calls ?? [];
  return (
    <View style={styles.assistant}>
      {message.reasoning ? <Reasoning text={message.reasoning} /> : null}
      {message.content.trim() ? <Markdown text={message.content} /> : null}
      {calls.map((c) => (
        <ToolActivityRow key={c.id} call={c} result={results.get(c.id)} running={running} />
      ))}
    </View>
  );
});

export function DraftBlock({ content, reasoning }: { content: string; reasoning: string }) {
  return (
    <View style={styles.assistant}>
      {reasoning ? <Reasoning text={reasoning} live={!content} /> : null}
      {content ? (
        <Markdown text={content} />
      ) : (
        <View style={styles.typing}>
          <ActivityIndicator size="small" color={colors.textMuted} />
          <Text style={styles.typingText}>{reasoning ? 'Thinking…' : 'Working…'}</Text>
        </View>
      )}
    </View>
  );
}

const styles = StyleSheet.create({
  userRow: { alignItems: 'flex-end', marginVertical: space.sm, paddingLeft: 48 },
  userBubble: {
    backgroundColor: colors.userBubble,
    borderRadius: radius.lg,
    borderBottomRightRadius: radius.sm,
    paddingHorizontal: space.md,
    paddingVertical: space.sm + 2,
  },
  userText: { color: colors.text, fontSize: 16, lineHeight: 22 },
  assistant: { marginVertical: space.sm, paddingRight: space.sm },
  reasoning: { marginBottom: space.xs },
  reasoningHeader: { minHeight: 32, justifyContent: 'center' },
  reasoningLabel: { color: colors.textFaint, fontSize: 14 },
  reasoningText: {
    color: colors.textMuted,
    fontSize: 14,
    lineHeight: 20,
    fontStyle: 'italic',
    borderLeftWidth: 2,
    borderLeftColor: colors.border,
    paddingLeft: space.sm,
    marginBottom: space.sm,
  },
  tool: {
    backgroundColor: colors.surface,
    borderRadius: radius.md,
    borderWidth: StyleSheet.hairlineWidth,
    borderColor: colors.border,
    marginVertical: 3,
    overflow: 'hidden',
  },
  toolHeader: { minHeight: 44, flexDirection: 'row', alignItems: 'center', paddingHorizontal: space.md, gap: space.sm },
  toolIcon: { fontSize: 14, fontWeight: '700', width: 20, textAlign: 'center' },
  toolSummary: { flex: 1, color: colors.text, fontSize: 14, fontFamily: monoFont },
  toolChevron: { color: colors.textFaint, fontSize: 14 },
  toolBody: {
    borderTopWidth: StyleSheet.hairlineWidth,
    borderTopColor: colors.border,
    padding: space.md,
    backgroundColor: colors.code,
  },
  toolSection: { color: colors.textFaint, fontSize: 11, textTransform: 'uppercase', marginBottom: 4, marginTop: 4 },
  toolMono: { color: colors.text, fontFamily: monoFont, fontSize: 12, lineHeight: 17 },
  typing: { flexDirection: 'row', alignItems: 'center', gap: space.sm, minHeight: 32 },
  typingText: { color: colors.textMuted, fontSize: 14 },
});
