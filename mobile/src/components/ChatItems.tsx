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
  web_search: 'magnify',
  fetch_url: 'web',
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

/**
 * Where Relay summarized older messages to fit the model's context. The
 * messages above stay here; tap to read what the model now has instead.
 */
export function SummaryDivider({ message }: { message: Message }) {
  const [open, setOpen] = useState(false);
  return (
    <View style={styles.summary}>
      <Pressable
        onPress={() => setOpen((o) => !o)}
        accessibilityRole="button"
        accessibilityState={{ expanded: open }}
        style={styles.summaryRow}
      >
        <View style={styles.summaryLine} />
        <Icon name="text-box-outline" size={16} color={m3.onSurfaceVariant} />
        <Text style={styles.summaryLabel}>Earlier messages summarized</Text>
        <Icon name={open ? 'chevron-up' : 'chevron-down'} size={16} color={m3.onSurfaceVariant} />
        <View style={styles.summaryLine} />
      </Pressable>
      {open ? (
        <View style={styles.summaryBody}>
          <Text style={styles.summaryNote}>
            The model's context was getting full, so it now sees this summary instead of the messages above.
            Everything is still saved here.
          </Text>
          <Markdown text={message.content} />
        </View>
      ) : null}
    </View>
  );
}

/** Shown while Relay writes a summary. */
export function CompactingRow() {
  return (
    <View style={[styles.summaryRow, styles.summary]}>
      <ActivityIndicator size="small" color={m3.primary} />
      <Text style={styles.summaryLabel}>Summarizing earlier messages to make room…</Text>
    </View>
  );
}

/**
 * A short status from the model's thinking so far: its latest complete
 * sentence (or line), trimmed. Shown on one line while it works, so the
 * person sees what it's doing without the whole stream racing past.
 */
export function thinkingStatus(text: string): string {
  const body = text.trim();
  const parts = body
    .split(/\n+/)
    .flatMap((line) => line.match(/.+?(?:[.!?]+(?=\s|$)|$)/g) ?? [])
    .map((x) =>
      x
        .replace(/^[\s#*>\-\d.)]+/, '')
        .replace(/[*_`]/g, '')
        .trim(),
    )
    .filter((x) => x.length > 1);
  // The last part may still be being written; prefer the one before it.
  const finished = /[.!?:*]$/.test(body) ? parts : parts.slice(0, -1);
  let pick = finished[finished.length - 1] ?? parts[parts.length - 1] ?? '';
  pick = pick.replace(/[.:]+$/, '');
  if (!pick) return 'Thinking…';
  if (pick.length > 90) pick = pick.slice(0, 88).replace(/[\s,;]+\S*$/, '');
  return /[?!]$/.test(pick) ? pick : `${pick}…`;
}

/** "Thought for 12s", from the seconds the model spent, if known. */
function thoughtLabel(seconds?: number): string {
  if (!seconds || !Number.isFinite(seconds) || seconds < 1) return 'Thought process';
  if (seconds < 60) return `Thought for ${Math.round(seconds)}s`;
  const m = Math.floor(seconds / 60);
  const s = Math.round(seconds % 60);
  return s ? `Thought for ${m}m ${s}s` : `Thought for ${m}m`;
}

/** The model's thinking, folded into one row you can open. */
function Reasoning({ text, seconds }: { text: string; seconds?: number }) {
  const [open, setOpen] = useState(false);
  const body = text.trim();
  if (!body) return null;
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
        <Text style={[type.bodyMedium, { color: m3.onSurfaceVariant }]}>{thoughtLabel(seconds)}</Text>
        <Icon name={open ? 'chevron-up' : 'chevron-down'} size={18} color={m3.outline} />
      </Pressable>
      {open ? (
        <Text selectable style={styles.reasoningText}>
          {body}
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
  | { kind: 'thought'; key: string; text: string; seconds?: number }
  | { kind: 'tool'; key: string; call: ToolCall };

/** When the model started on each reply: the time of the message before it. */
const startedAt = new WeakMap<Message, string>();
export function noteStart(m: Message, previous: Message | undefined) {
  if (previous) startedAt.set(m, previous.created_at);
}
function secondsBefore(m: Message): number | undefined {
  const from = startedAt.get(m);
  if (!from) return undefined;
  const s = (Date.parse(m.created_at) - Date.parse(from)) / 1000;
  return Number.isFinite(s) && s >= 0 ? s : undefined;
}

/** "Ran 2 commands · read 3 files" style summary of a turn's tool use. */
function stepsSummary(calls: ToolCall[]): string {
  const n = (...names: string[]) => calls.filter((c) => names.includes(c.name)).length;
  const plural = (count: number, word: string) => `${count} ${word}${count === 1 ? '' : 's'}`;
  const parts: string[] = [];
  const known = n('run_command', 'read_file', 'edit_file', 'write_file', 'list_dir', 'web_search', 'fetch_url');
  if (n('web_search')) parts.push(`searched the web ${n('web_search') === 1 ? 'once' : `${n('web_search')} times`}`);
  if (n('fetch_url')) parts.push(`read ${plural(n('fetch_url'), 'page')}`);
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
  // Follows the run (open while working, folded once done) until tapped.
  const [userOpen, setUserOpen] = useState<boolean | null>(null);
  const open = userOpen ?? running;
  const last = messages[messages.length - 1];
  const lastIsAnswer = !!last && !running && !last.tool_calls?.length && !!last.content.trim();
  const work = lastIsAnswer ? messages.slice(0, -1) : messages;
  const steps: Step[] = [];
  const calls: ToolCall[] = [];
  for (const m of work) {
    if (m.reasoning?.trim())
      steps.push({ kind: 'thought', key: `r${m.id}`, text: m.reasoning, seconds: secondsBefore(m) });
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
        ? thinkingStatus(draft.reasoning)
        : 'Working…'
      : null;
  const answerSeconds = lastIsAnswer ? secondsBefore(last) : undefined;
  const showSteps = calls.length > 0 || (steps.length > 0 && !answer);
  // One fold-out for all the work, including the thinking behind the answer.
  if (showSteps && answerReasoning?.trim())
    steps.push({ kind: 'thought', key: 'answer-r', text: answerReasoning, seconds: answerSeconds });

  return (
    <View style={styles.turn}>
      {showSteps ? (
        <View>
          <Pressable
            onPress={() => setUserOpen(!open)}
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
                  <Reasoning key={st.key} text={st.text} seconds={st.seconds} />
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
      {!showSteps && answerReasoning ? <Reasoning text={answerReasoning} seconds={answerSeconds} /> : null}
      {answer ? <Markdown text={answer} /> : null}
      {!showSteps && !answer && running ? (
        <View style={styles.typing}>
          <ActivityIndicator size="small" color={m3.primary} />
          <Text style={[type.bodyMedium, styles.stepsText]} numberOfLines={1}>
            {draft?.reasoning ? thinkingStatus(draft.reasoning) : 'Working…'}
          </Text>
        </View>
      ) : null}
    </View>
  );
});

export function DraftBlock({ content, reasoning }: { content: string; reasoning: string }) {
  return (
    <View style={styles.assistant}>
      {reasoning && content ? <Reasoning text={reasoning} /> : null}
      {content ? (
        <Markdown text={content} />
      ) : (
        <View style={styles.typing}>
          <ActivityIndicator size="small" color={m3.primary} />
          <Text style={[type.bodyMedium, styles.stepsText]} numberOfLines={1}>
            {reasoning ? thinkingStatus(reasoning) : 'Working…'}
          </Text>
        </View>
      )}
    </View>
  );
}

const styles = StyleSheet.create({
  summary: { marginVertical: space.lg },
  summaryRow: { flexDirection: 'row', alignItems: 'center', justifyContent: 'center', gap: space.sm, minHeight: 36 },
  summaryLine: { flex: 1, height: StyleSheet.hairlineWidth, backgroundColor: m3.outlineVariant },
  summaryLabel: { ...type.labelLarge, color: m3.onSurfaceVariant },
  summaryBody: {
    marginTop: space.sm,
    padding: space.lg,
    borderRadius: radius.lg,
    backgroundColor: m3.surfaceContainer,
    gap: space.sm,
  },
  summaryNote: { ...type.bodySmall, color: m3.onSurfaceVariant },
  userRow: { alignItems: 'flex-end', marginTop: space.xl, marginBottom: space.sm, paddingLeft: 56 },
  userBubble: {
    backgroundColor: m3.surfaceContainerHigh,
    borderRadius: radius.lg,
    paddingHorizontal: space.lg,
    paddingVertical: space.md,
  },
  userText: { ...type.bodyLarge, color: m3.onSurface },
  assistant: { marginVertical: space.md, gap: space.sm },
  reasoning: { paddingVertical: 4 },
  thoughtText: { ...type.bodyMedium, color: m3.onSurfaceVariant, fontStyle: 'italic' },
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
