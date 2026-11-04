import { type ComponentRef, memo, useEffect, useRef } from 'react';
import { ScrollView, StyleSheet, Text, useWindowDimensions, View } from 'react-native';

import type { FileChange, Line } from '../lib/liveCode';
import { m3, mono, radius, space, type } from './theme';
import { Icon } from './ui';

/** Most lines drawn at once; a longer change shows its end while live. */
const MAX_LINES = 600;

function fileName(path: string): string {
  const parts = path.split(/[\\/]/);
  return parts[parts.length - 1] || path;
}

function Counts({ added, removed }: { added: number; removed: number }) {
  return (
    <Text style={styles.counts}>
      {added ? <Text style={{ color: m3.success }}>+{added}</Text> : null}
      {added && removed ? ' ' : null}
      {removed ? <Text style={{ color: m3.error }}>−{removed}</Text> : null}
    </Text>
  );
}

const LineRow = memo(function LineRow({ line, cursor }: { line: Line; cursor?: boolean }) {
  const sign = line.kind === 'add' ? '+' : line.kind === 'del' ? '−' : ' ';
  return (
    <View style={[styles.line, line.kind === 'add' && styles.add, line.kind === 'del' && styles.del]}>
      <Text style={[styles.sign, line.kind === 'add' && { color: m3.success }, line.kind === 'del' && { color: m3.error }]}>
        {sign}
      </Text>
      <Text selectable style={[styles.code, line.kind === 'same' && { color: m3.onSurfaceVariant }]}>
        {line.text || ' '}
        {cursor ? <Text style={styles.cursor}>▍</Text> : null}
      </Text>
    </View>
  );
});

/**
 * A file change as lines: added in green, removed in red. With `live`, it
 * follows the end of the text as the model writes it, with a cursor.
 */
export function CodeChange({ change, live, maxHeight }: { change: FileChange; live?: boolean; maxHeight?: number }) {
  const scroll = useRef<ComponentRef<typeof ScrollView>>(null);
  const { width, height } = useWindowDimensions();
  const phone = width < 600;
  const max = maxHeight ?? (phone ? Math.min(240, height * 0.35) : 360);
  const lines = change.lines;
  const from = live ? Math.max(0, lines.length - MAX_LINES) : 0;
  const shown = live ? lines.slice(from) : lines.slice(0, MAX_LINES);
  const hidden = lines.length - shown.length;
  const verb = live
    ? change.tool === 'write_file'
      ? 'Writing'
      : 'Editing'
    : change.tool === 'write_file'
      ? 'Wrote'
      : 'Edited';

  useEffect(() => {
    if (live) scroll.current?.scrollToEnd({ animated: false });
  }, [live, lines.length, lines[lines.length - 1]?.text]);

  return (
    <View style={styles.card}>
      <View style={styles.head}>
        <Icon name={change.tool === 'write_file' ? 'file-plus-outline' : 'file-edit-outline'} size={16} color={m3.onSurfaceVariant} />
        <Text style={[type.labelLarge, styles.path]} numberOfLines={1}>
          {verb} <Text style={{ color: m3.onSurface }}>{change.path ? fileName(change.path) : 'a file'}</Text>
        </Text>
        <Counts added={change.added} removed={change.removed} />
      </View>
      {change.path && change.path !== fileName(change.path) ? (
        <Text style={styles.fullPath} numberOfLines={1}>
          {change.path}
        </Text>
      ) : null}
      {shown.length ? (
        <ScrollView ref={scroll} style={{ maxHeight: max }} nestedScrollEnabled>
          {live && hidden ? <Text style={styles.more}>{hidden} earlier lines</Text> : null}
          <ScrollView horizontal nestedScrollEnabled contentContainerStyle={styles.lines}>
            <View style={styles.grow}>
              {shown.map((l, i) => (
                <LineRow key={from + i} line={l} cursor={live && i === shown.length - 1} />
              ))}
            </View>
          </ScrollView>
          {!live && hidden ? <Text style={styles.more}>{hidden} more lines</Text> : null}
        </ScrollView>
      ) : (
        <Text style={styles.more}>{live ? 'Starting…' : 'No lines.'}</Text>
      )}
    </View>
  );
}

/** "3 files changed · +120 −8" for a turn's file changes. */
export function changesSummary(changes: FileChange[]): { files: number; added: number; removed: number } | null {
  if (!changes.length) return null;
  const files = new Set(changes.map((c) => c.path)).size;
  return {
    files,
    added: changes.reduce((s, c) => s + c.added, 0),
    removed: changes.reduce((s, c) => s + c.removed, 0),
  };
}

export function ChangesLine({ changes }: { changes: FileChange[] }) {
  const s = changesSummary(changes);
  if (!s) return null;
  return (
    <View style={styles.summary}>
      <Icon name="file-document-multiple-outline" size={16} color={m3.outline} />
      <Text style={[type.bodyMedium, { color: m3.onSurfaceVariant }]}>
        {s.files} {s.files === 1 ? 'file' : 'files'} changed
      </Text>
      <Counts added={s.added} removed={s.removed} />
    </View>
  );
}

const styles = StyleSheet.create({
  card: {
    borderRadius: radius.md,
    borderWidth: 1,
    borderColor: m3.outlineVariant,
    backgroundColor: m3.code,
    overflow: 'hidden',
    marginVertical: space.xs,
  },
  head: {
    flexDirection: 'row',
    alignItems: 'center',
    gap: space.sm,
    paddingHorizontal: space.md,
    paddingTop: space.sm,
    paddingBottom: 4,
  },
  path: { flex: 1, minWidth: 0, color: m3.onSurfaceVariant },
  fullPath: { ...type.bodySmall, color: m3.outline, paddingHorizontal: space.md, paddingBottom: 4, fontFamily: mono },
  counts: { fontFamily: mono, fontSize: 12 },
  lines: { paddingVertical: 4, minWidth: '100%', flexGrow: 1 },
  grow: { flexGrow: 1 },
  line: { flexDirection: 'row', paddingRight: space.md },
  add: { backgroundColor: 'rgba(157,220,156,0.10)' },
  del: { backgroundColor: 'rgba(255,180,171,0.10)' },
  sign: { width: 22, textAlign: 'center', fontFamily: mono, fontSize: 12, lineHeight: 18, color: m3.outline },
  code: { fontFamily: mono, fontSize: 12, lineHeight: 18, color: m3.onSurface },
  cursor: { color: m3.primary },
  more: { ...type.bodySmall, color: m3.outline, paddingHorizontal: space.md, paddingVertical: 4 },
  summary: { flexDirection: 'row', alignItems: 'center', gap: space.sm, minHeight: 28 },
});
