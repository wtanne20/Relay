import { memo, useState } from 'react';
import { ActivityIndicator, Pressable, ScrollView, StyleSheet, Text, View } from 'react-native';

import { toolDetails, toolSummary } from '../lib/format';
import type { Message, ToolCall } from '../lib/types';
import { Markdown } from './Markdown';
import { m3, mono as monoFont, radius, space, type } from './theme';
import { Icon, type IconName } from './ui';

const MAX_RESULT_CHARS = 6000;

const TOOL_ICON: Record<string, IconName> = {
  run_command: 'console',
  read_file: 'file-document-outline',
  write_file: 'file-plus-outline',
  edit_file: 'file-edit-outline',
  list_dir: 'folder-outline',
};

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
        <Icon name="lightbulb-outline" size={16} color={m3.onSurfaceVariant} />
        <Text style={[type.labelLarge, { color: m3.onSurfaceVariant }]}>{live ? 'Thinking…' : 'Thought process'}</Text>
        <Icon name={open ? 'chevron-up' : 'chevron-down'} size={18} color={m3.outline} />
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
  const iconColor = pending ? m3.primary : failed ? m3.error : m3.success;
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
        <View style={styles.toolIconWrap}>
          <Icon name={TOOL_ICON[call.name] ?? 'wrench-outline'} size={18} color={m3.onSecondaryContainer} />
        </View>
        <Text
          style={[type.bodyMedium, styles.toolSummary, failed && { color: m3.error }]}
          numberOfLines={open ? undefined : 1}
        >
          {summary}
        </Text>
        {pending && running ? (
          <ActivityIndicator size="small" color={iconColor} />
        ) : (
          <Icon
            name={failed ? 'close-circle-outline' : pending ? 'circle-outline' : 'check-circle-outline'}
            size={18}
            color={iconColor}
          />
        )}
        <Icon name={open ? 'chevron-up' : 'chevron-down'} size={18} color={m3.outline} />
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
          <ActivityIndicator size="small" color={m3.primary} />
          <Text style={[type.bodyMedium, { color: m3.onSurfaceVariant }]}>{reasoning ? 'Thinking…' : 'Working…'}</Text>
        </View>
      )}
    </View>
  );
}

const styles = StyleSheet.create({
  userRow: { alignItems: 'flex-end', marginVertical: space.md, paddingLeft: 56 },
  userBubble: {
    backgroundColor: m3.surfaceContainerHighest,
    borderRadius: 22,
    borderBottomRightRadius: 6,
    paddingHorizontal: space.lg,
    paddingVertical: space.md,
  },
  userText: { ...type.bodyLarge, color: m3.onSurface },
  assistant: { marginVertical: space.md, gap: space.sm },
  reasoning: {},
  reasoningHeader: {
    alignSelf: 'flex-start',
    minHeight: 32,
    flexDirection: 'row',
    alignItems: 'center',
    gap: 6,
    paddingHorizontal: space.md,
    borderRadius: radius.sm,
    backgroundColor: m3.surfaceContainerLow,
  },
  reasoningText: {
    ...type.bodyMedium,
    color: m3.onSurfaceVariant,
    borderLeftWidth: 2,
    borderLeftColor: m3.outlineVariant,
    paddingLeft: space.md,
    marginTop: space.sm,
  },
  tool: { backgroundColor: m3.surfaceContainerLow, borderRadius: radius.lg, overflow: 'hidden' },
  toolHeader: {
    minHeight: 52,
    flexDirection: 'row',
    alignItems: 'center',
    paddingHorizontal: space.sm,
    paddingRight: space.md,
    gap: space.sm,
  },
  toolIconWrap: {
    width: 32,
    height: 32,
    borderRadius: 10,
    backgroundColor: m3.secondaryContainer,
    alignItems: 'center',
    justifyContent: 'center',
  },
  toolSummary: { flex: 1, minWidth: 0, color: m3.onSurface },
  toolBody: { padding: space.md, paddingTop: 0, gap: 4 },
  toolSection: { ...type.labelMedium, color: m3.outline, textTransform: 'uppercase', marginTop: space.sm },
  toolMono: {
    color: m3.onSurface,
    fontFamily: monoFont,
    fontSize: 12,
    lineHeight: 18,
    backgroundColor: m3.code,
    borderRadius: radius.sm,
    padding: space.sm,
  },
  typing: { flexDirection: 'row', alignItems: 'center', gap: space.sm, minHeight: 32 },
});
