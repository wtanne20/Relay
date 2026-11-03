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
    <View>
      <Pressable
        onPress={() => setOpen((o) => !o)}
        style={({ pressed }) => [styles.toolHeader, pressed && { opacity: 0.7 }]}
        accessibilityRole="button"
        accessibilityLabel={summary}
        accessibilityState={{ expanded: open }}
      >
        <Icon name={TOOL_ICON[call.name] ?? 'wrench-outline'} size={16} color={m3.onSurfaceVariant} />
        <Text
          style={[type.bodyMedium, styles.toolSummary, failed && { color: m3.error }]}
          numberOfLines={open ? undefined : 1}
        >
          {summary}
        </Text>
        {pending && running ? (
          <ActivityIndicator size="small" color={iconColor} />
        ) : failed ? (
          <Icon name="close-circle-outline" size={16} color={iconColor} />
        ) : null}
        <Icon name={open ? 'chevron-up' : 'chevron-down'} size={16} color={m3.outline} />
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

type Step =
  | { kind: 'text'; key: string; text: string }
  | { kind: 'thought'; key: string; text: string }
  | { kind: 'tool'; key: string; call: ToolCall };

/** "Ran 2 commands · read 3 files" style summary of a turn's tool use. */
function stepsSummary(calls: ToolCall[]): string {
  const n = (...names: string[]) => calls.filter((c) => names.includes(c.name)).length;
  const plural = (count: number, word: string) => `${count} ${word}${count === 1 ? '' : 's'}`;
  const parts: string[] = [];
  const known = n('run_command', 'read_file', 'edit_file', 'write_file', 'list_dir');
  if (n('run_command')) parts.push(`ran ${plural(n('run_command'), 'command')}`);
  if (n('read_file')) parts.push(`read ${plural(n('read_file'), 'file')}`);
  if (n('edit_file', 'write_file')) parts.push(`changed ${plural(n('edit_file', 'write_file'), 'file')}`);
  if (n('list_dir')) parts.push(`listed ${plural(n('list_dir'), 'folder')}`);
  if (calls.length > known) parts.push(plural(calls.length - known, 'other step'));
  const text = parts.join(' · ') || 'Thought process';
  return text.charAt(0).toUpperCase() + text.slice(1);
}

/**
 * One agent reply, Claude-style: the work it did (thinking, tool calls and
 * in-between notes) folded into one quiet "steps" line that expands, and the
 * final answer as plain full-width text below it.
 */
export const AssistantTurn = memo(function AssistantTurn({
  messages,
  results,
  running,
  draft,
}: {
  messages: Message[];
  results: Map<string, Message>;
  running: boolean;
  draft?: { content: string; reasoning: string } | null;
}) {
  const [open, setOpen] = useState(false);
  const last = messages[messages.length - 1];
  const lastIsAnswer = !!last && !running && !last.tool_calls?.length && !!last.content.trim();
  const work = lastIsAnswer ? messages.slice(0, -1) : messages;
  const steps: Step[] = [];
  const calls: ToolCall[] = [];
  for (const m of work) {
    if (m.reasoning?.trim()) steps.push({ kind: 'thought', key: `r${m.id}`, text: m.reasoning });
    if (m.content.trim()) steps.push({ kind: 'text', key: `t${m.id}`, text: m.content });
    for (const c of m.tool_calls ?? []) {
      steps.push({ kind: 'tool', key: c.id, call: c });
      calls.push(c);
    }
  }
  const pendingCall = running ? calls.find((c) => !results.get(c.id)) : undefined;
  const answer = lastIsAnswer ? last.content : running && draft?.content ? draft.content : '';
  const answerReasoning = lastIsAnswer ? last.reasoning : running && draft?.content ? draft.reasoning : '';
  const liveStatus = pendingCall
    ? toolSummary(pendingCall, undefined, true)
    : running && !draft?.content
      ? draft?.reasoning
        ? 'Thinking…'
        : 'Working…'
      : null;
  const showSteps = calls.length > 0 || (steps.length > 0 && !answer);
  // One fold-out for all the work, including the thinking behind the answer.
  if (showSteps && answerReasoning?.trim()) steps.push({ kind: 'thought', key: 'answer-r', text: answerReasoning });

  return (
    <View style={styles.turn}>
      {showSteps ? (
        <View>
          <Pressable
            onPress={() => setOpen((o) => !o)}
            style={({ pressed, hovered }: { pressed: boolean; hovered?: boolean }) => [
              styles.stepsHeader,
              (pressed || hovered) && { backgroundColor: m3.hover },
            ]}
            accessibilityRole="button"
            accessibilityState={{ expanded: open }}
          >
            {running ? (
              <ActivityIndicator size="small" color={m3.primary} />
            ) : (
              <Icon name="check-circle-outline" size={16} color={m3.outline} />
            )}
            <Text style={[type.bodyMedium, styles.stepsText]} numberOfLines={1}>
              {liveStatus ?? stepsSummary(calls)}
            </Text>
            <Icon name={open ? 'chevron-up' : 'chevron-down'} size={18} color={m3.outline} />
          </Pressable>
          {open ? (
            <View style={styles.stepsList}>
              {steps.map((st) =>
                st.kind === 'tool' ? (
                  <ToolActivityRow key={st.key} call={st.call} result={results.get(st.call.id)} running={running} />
                ) : st.kind === 'thought' ? (
                  <Reasoning key={st.key} text={st.text} />
                ) : (
                  <Text key={st.key} selectable style={[type.bodyMedium, styles.stepNote]}>
                    {st.text.trim()}
                  </Text>
                ),
              )}
            </View>
          ) : null}
        </View>
      ) : null}
      {!showSteps && answerReasoning ? <Reasoning text={answerReasoning} /> : null}
      {answer ? <Markdown text={answer} /> : null}
      {!showSteps && !answer && running ? (
        <View style={styles.typing}>
          <ActivityIndicator size="small" color={m3.primary} />
          <Text style={[type.bodyMedium, { color: m3.onSurfaceVariant }]}>
            {draft?.reasoning ? 'Thinking…' : 'Working…'}
          </Text>
        </View>
      ) : null}
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
  userRow: { alignItems: 'flex-end', marginTop: space.xl, marginBottom: space.sm, paddingLeft: 56 },
  userBubble: {
    backgroundColor: m3.surfaceContainerHigh,
    borderRadius: radius.lg,
    paddingHorizontal: space.lg,
    paddingVertical: space.md,
  },
  userText: { ...type.bodyLarge, color: m3.onSurface },
  assistant: { marginVertical: space.md, gap: space.sm },
  reasoning: {},
  reasoningHeader: {
    alignSelf: 'flex-start',
    minHeight: 28,
    flexDirection: 'row',
    alignItems: 'center',
    gap: 6,
    borderRadius: radius.sm,
  },
  reasoningText: {
    ...type.bodyMedium,
    color: m3.onSurfaceVariant,
    borderLeftWidth: 2,
    borderLeftColor: m3.outlineVariant,
    paddingLeft: space.md,
    marginTop: space.sm,
  },
  toolHeader: {
    minHeight: 30,
    flexDirection: 'row',
    alignItems: 'center',
    gap: space.sm,
  },
  toolSummary: { flex: 1, minWidth: 0, color: m3.onSurfaceVariant },
  toolBody: { paddingLeft: space.xl, paddingBottom: space.sm, gap: 4 },
  turn: { marginTop: space.md, marginBottom: space.md, gap: space.sm },
  stepsHeader: {
    alignSelf: 'flex-start',
    maxWidth: '100%',
    minHeight: 32,
    flexDirection: 'row',
    alignItems: 'center',
    gap: space.sm,
    paddingHorizontal: space.sm,
    marginLeft: -space.sm,
    borderRadius: radius.sm,
  },
  stepsText: { color: m3.onSurfaceVariant, flexShrink: 1 },
  stepsList: {
    marginTop: space.xs,
    marginLeft: space.xs,
    paddingLeft: space.md,
    borderLeftWidth: 1,
    borderLeftColor: m3.outlineVariant,
    gap: 2,
  },
  stepNote: { color: m3.onSurfaceVariant, paddingVertical: 4 },
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
