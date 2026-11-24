import { memo, useState } from 'react';
import { ActivityIndicator, Pressable, ScrollView, StyleSheet, Text, View } from 'react-native';

import { toolDetails, toolSummary } from '../lib/format';
import { fileChange, partialArgs, readArgs, type FileChange } from '../lib/liveCode';
import type { Draft, Message, ReplyStats, ToolCall } from '../lib/types';
import { MessageFiles, MessagePicture } from './Attachments';
import { ChangesLine, CodeChange } from './CodeChange';
import { Markdown } from './Markdown';
import { trimPartialMarkdown, useSmoothText } from '../lib/useSmoothText';
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
  search_documents: 'file-search-outline',
  read_document: 'file-document-outline',
};

export function UserBubble({ message }: { message: Message }) {
  const pictures = message.images ?? [];
  return (
    <View style={styles.userRow}>
      {pictures.length ? (
        <View style={styles.userPictures}>
          {pictures.map((p) => (
            <MessagePicture key={p} name={p} />
          ))}
        </View>
      ) : null}
      <MessageFiles files={message.files ?? []} />
      {message.content.trim() ? (
        <View style={styles.userBubble}>
          <Text selectable style={styles.userText}>
            {message.content}
          </Text>
        </View>
      ) : null}
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

/**
 * Relay carried on a reply that was cut off when the model's context filled
 * up. Tap to see what it told the model.
 */
export function ContinueDivider({ message }: { message: Message }) {
  const [open, setOpen] = useState(false);
  const looped = /loop/i.test(message.content.split('\n')[0] ?? '');
  return (
    <View style={styles.summary}>
      <Pressable
        onPress={() => setOpen((o) => !o)}
        accessibilityRole="button"
        accessibilityState={{ expanded: open }}
        style={styles.summaryRow}
      >
        <View style={styles.summaryLine} />
        <Icon name={looped ? 'sync-alert' : 'play-circle-outline'} size={16} color={m3.onSurfaceVariant} />
        <Text style={styles.summaryLabel}>
          {looped ? 'Reply was going in circles, restarted it' : 'Reply was cut off, continued it'}
        </Text>
        <Icon name={open ? 'chevron-up' : 'chevron-down'} size={16} color={m3.onSurfaceVariant} />
        <View style={styles.summaryLine} />
      </Pressable>
      {open ? (
        <View style={styles.summaryBody}>
          <Text style={styles.summaryNote}>
            The model ran out of context in the middle of its reply, so Relay told it to carry on. This is what it
            was sent.
          </Text>
          <Markdown text={message.content.replace(/^\(Relay\) /, '')} />
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

export function LoadingModelRow() {
  return (
    <View style={[styles.summaryRow, styles.summary]}>
      <ActivityIndicator size="small" color={m3.primary} />
      <Text style={styles.summaryLabel}>Loading the model, this takes a few seconds…</Text>
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
  const change = open ? fileChange(call.name, readArgs(call.arguments)) : null;
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
          {change ? (
            <CodeChange change={change} />
          ) : (
            <>
              <Text style={styles.toolSection}>{call.name}</Text>
              <ScrollView horizontal nestedScrollEnabled>
                <Text selectable style={styles.toolMono}>
                  {toolDetails(call)}
                </Text>
              </ScrollView>
            </>
          )}
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
  const known = n(
    'run_command',
    'read_file',
    'edit_file',
    'write_file',
    'list_dir',
    'web_search',
    'fetch_url',
    'search_documents',
    'read_document',
  );
  if (n('web_search')) parts.push(`searched the web ${n('web_search') === 1 ? 'once' : `${n('web_search')} times`}`);
  if (n('fetch_url')) parts.push(`read ${plural(n('fetch_url'), 'page')}`);
  if (n('search_documents'))
    parts.push(`searched your documents ${n('search_documents') === 1 ? 'once' : `${n('search_documents')} times`}`);
  if (n('read_document')) parts.push(`read ${plural(n('read_document'), 'document')}`);
  if (n('run_command')) parts.push(`ran ${plural(n('run_command'), 'command')}`);
  if (n('read_file')) parts.push(`read ${plural(n('read_file'), 'file')}`);
  // Files, not attempts: several edits to one file count once.
  const changed = new Set(
    calls.filter((c) => c.name === 'edit_file' || c.name === 'write_file').map((c) => readArgs(c.arguments).path ?? c.id),
  ).size;
  if (changed) parts.push(`changed ${plural(changed, 'file')}`);
  if (n('list_dir')) parts.push(`listed ${plural(n('list_dir'), 'folder')}`);
  if (calls.length > known) parts.push(plural(calls.length - known, 'other step'));
  const text = parts.join(' · ') || 'Thought process';
  return text.charAt(0).toUpperCase() + text.slice(1);
}

type WorkPart =
  | { kind: 'thought'; key: string; text: string; seconds?: number }
  | { kind: 'tool'; key: string; call: ToolCall };

/**
 * A reply in the order it was written: text the model wrote, and runs of
 * work between (thinking and tool calls) that fold into one line.
 */
type Segment =
  | { kind: 'text'; key: string; text: string; live: boolean }
  | { kind: 'work'; key: string; parts: WorkPart[] };

function segmentsOf(messages: Message[], draft: Draft | null | undefined): Segment[] {
  const out: Segment[] = [];
  const work = (part: WorkPart) => {
    const tail = out[out.length - 1];
    if (tail?.kind === 'work') tail.parts.push(part);
    else out.push({ kind: 'work', key: `s${out.length}`, parts: [part] });
  };
  const text = (t: string, live: boolean) => {
    if (t.trim()) out.push({ kind: 'text', key: `s${out.length}`, text: t, live });
  };
  for (const m of messages) {
    if (m.reasoning?.trim()) work({ kind: 'thought', key: `r${m.id}`, text: m.reasoning, seconds: secondsBefore(m) });
    text(m.content, false);
    for (const c of m.tool_calls ?? []) work({ kind: 'tool', key: c.id, call: c });
  }
  if (draft) {
    if (draft.reasoning.trim()) work({ kind: 'thought', key: 'draft-r', text: draft.reasoning });
    text(draft.content, true);
    // A step the model is still writing: its line shows what it is (and a
    // file being written shows live below it) until the step is saved.
    if (draft.tool_calls?.some((c) => c.name) && out[out.length - 1]?.kind !== 'work')
      out.push({ kind: 'work', key: `s${out.length}`, parts: [] });
  }
  return out;
}

/**
 * Text as the model writes it: in the full reading style from the first
 * word, flowing in at an even pace instead of in bursts, with half-written
 * Markdown held back until it's complete.
 */
const ReplyText = memo(function ReplyText({ text, live, writing }: { text: string; live: boolean; writing: boolean }) {
  const shown = useSmoothText(text, live);
  const flowing = writing || shown.length < text.length;
  return <Markdown text={flowing ? trimPartialMarkdown(shown) : text} />;
});

/** One run of work between pieces of text, folded into a line you can open. */
function WorkGroup({
  parts,
  results,
  running,
  active,
  status,
  live,
  liveWriting,
}: {
  parts: WorkPart[];
  results: Map<string, Message>;
  running: boolean;
  /** The step the agent is on right now. */
  active: boolean;
  status: string;
  live: FileChange | null;
  liveWriting: boolean;
}) {
  // Follows the work (open while it's going on, folded after) until tapped.
  const [userOpen, setUserOpen] = useState<boolean | null>(null);
  const open = userOpen ?? active;
  const calls = parts.flatMap((p) => (p.kind === 'tool' ? [p.call] : []));
  const thoughts = parts.filter((p) => p.kind === 'thought');
  if (!active && !calls.length) {
    // Only thinking: the usual "Thought for 12s" row.
    const seconds = thoughts.reduce<number | undefined>(
      (sum, p) => (p.kind === 'thought' && p.seconds !== undefined ? (sum ?? 0) + p.seconds : sum),
      undefined,
    );
    return <Reasoning text={thoughts.map((p) => (p.kind === 'thought' ? p.text.trim() : '')).join('\n\n')} seconds={seconds} />;
  }
  return (
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
        {active ? (
          <ActivityIndicator size="small" color={m3.primary} />
        ) : (
          <Icon name="check-circle-outline" size={16} color={m3.outline} />
        )}
        <Text style={[type.bodyMedium, styles.stepsText]} numberOfLines={1}>
          {active ? status : stepsSummary(calls)}
        </Text>
        {parts.length ? <Icon name={open ? 'chevron-up' : 'chevron-down'} size={18} color={m3.outline} /> : null}
      </Pressable>
      {open && parts.length ? (
        <View style={styles.stepsList}>
          {parts.map((p) =>
            p.kind === 'tool' ? (
              <ToolActivityRow key={p.key} call={p.call} result={results.get(p.call.id)} running={running} />
            ) : (
              <Reasoning key={p.key} text={p.text} seconds={p.seconds} />
            ),
          )}
        </View>
      ) : null}
      {active && live ? <CodeChange change={live} live={liveWriting} /> : null}
    </View>
  );
}

/**
 * One agent reply, Claude-style: text in the full reading style as it's
 * written, with the work between it (thinking, tool calls) folded into quiet
 * lines that expand. Nothing moves or changes size when the reply finishes.
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
  draft?: Draft | null;
}) {
  const segments = segmentsOf(messages, running ? draft : null);
  const calls = messages.flatMap((m) => m.tool_calls ?? []);
  const pendingCall = running ? calls.find((c) => !results.get(c.id)) : undefined;
  // The file the model is writing right now, or the one waiting to be saved.
  let live: FileChange | null = null;
  if (running) {
    const writing = (draft?.tool_calls ?? []).filter((c) => c.name === 'write_file' || c.name === 'edit_file');
    const cur = writing[writing.length - 1];
    if (cur) live = fileChange(cur.name, partialArgs(cur.arguments));
    else if (pendingCall) live = fileChange(pendingCall.name, readArgs(pendingCall.arguments));
  }
  const liveWriting = !!live && !pendingCall;
  const draftCall = (draft?.tool_calls ?? []).filter((c) => c.name).pop();
  const status =
    liveWriting && live
      ? `${live.tool === 'write_file' ? 'Writing' : 'Editing'} ${live.path ? live.path.split(/[\\/]/).pop() : 'a file'}…`
      : pendingCall
        ? toolSummary(pendingCall, undefined, true)
        : draftCall
          ? toolSummary({ id: 'draft', ...draftCall }, undefined, true)
          : draft?.reasoning && !draft.content
            ? thinkingStatus(draft.reasoning)
            : 'Working…';
  const tail = segments[segments.length - 1];
  const changes = running
    ? []
    : calls
        .filter((c) => results.get(c.id) && !results.get(c.id)?.is_error)
        .map((c) => fileChange(c.name, readArgs(c.arguments)))
        .filter((c): c is FileChange => !!c);
  const last = messages[messages.length - 1];

  return (
    <View style={styles.turn}>
      {segments.map((seg) =>
        seg.kind === 'text' ? (
          <ReplyText key={seg.key} text={seg.text} live={seg.live} writing={running && seg === tail} />
        ) : (
          <WorkGroup
            key={seg.key}
            parts={seg.parts}
            results={results}
            running={running}
            active={running && seg === tail}
            status={status}
            live={live}
            liveWriting={liveWriting}
          />
        ),
      )}
      {running && !tail ? (
        <View style={styles.typing}>
          <ActivityIndicator size="small" color={m3.primary} />
          <Text style={[type.bodyMedium, styles.stepsText]} numberOfLines={1}>
            Working…
          </Text>
        </View>
      ) : null}
      {changes.length ? <ChangesLine changes={changes} /> : null}
      {!running && last && !last.tool_calls?.length && last.stats ? <ReplyMeta stats={last.stats} /> : null}
    </View>
  );
});

/** "Qwen3-4B · 52 tok/s · quick": which model answered, and how fast. */
function ReplyMeta({ stats }: { stats: ReplyStats }) {
  const parts = [stats.model.replace(/\.gguf$/i, '')];
  if (stats.tokens_per_sec) parts.push(`${Math.round(stats.tokens_per_sec)} tok/s`);
  if (stats.route) parts.push(stats.route === 'quick' ? 'quick question' : 'big model');
  return (
    <View style={styles.replyMeta} accessibilityLabel={`Answered by ${parts.join(', ')}`}>
      <Icon name={stats.route === 'quick' ? 'lightning-bolt-outline' : 'chip'} size={13} color={m3.outline} />
      <Text style={[type.bodySmall, styles.replyMetaText]} numberOfLines={1}>
        {parts.join(' · ')}
      </Text>
    </View>
  );
}

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
  replyMeta: { flexDirection: 'row', alignItems: 'center', gap: 4, marginTop: space.xs },
  replyMetaText: { color: m3.outline, flexShrink: 1 },
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
  userPictures: { alignItems: 'flex-end', gap: space.sm, marginBottom: space.sm, alignSelf: 'stretch' },
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
