import { memo, useMemo, type ReactNode } from 'react';
import { Linking, Platform, Pressable, ScrollView, StyleSheet, Text, View } from 'react-native';

import { MockupCard, isMockup } from './Mockup';
import { colors, radius, space } from './theme';
import { Icon } from './ui';

/**
 * A small Markdown renderer covering what coding agents produce: fenced code
 * blocks, headings, bullet/numbered lists (one level of nesting), tables,
 * block quotes, paragraphs, and inline `code`, **bold**, *italic*,
 * [links](url) and bare https:// links. Streaming-safe: an unterminated code
 * fence renders as code.
 */

type Block =
  | { kind: 'code'; lang: string; text: string; closed: boolean }
  | { kind: 'heading'; level: number; text: string }
  | { kind: 'list'; ordered: boolean; items: string[]; start: number }
  | { kind: 'quote'; text: string }
  | { kind: 'rule' }
  | { kind: 'table'; head: string[]; rows: string[][] }
  | { kind: 'para'; text: string };

/** "| a | b |" -> ["a", "b"]; pipes inside `code` are kept. */
function cells(line: string): string[] {
  let t = line.trim();
  if (t.startsWith('|')) t = t.slice(1);
  if (t.endsWith('|') && !t.endsWith('\\|')) t = t.slice(0, -1);
  const out: string[] = [];
  let cur = '';
  let code = false;
  for (let i = 0; i < t.length; i++) {
    const c = t[i];
    if (c === '`') code = !code;
    if (c === '\\' && t[i + 1] === '|') {
      cur += '|';
      i++;
    } else if (c === '|' && !code) {
      out.push(cur.trim());
      cur = '';
    } else {
      cur += c;
    }
  }
  out.push(cur.trim());
  return out;
}

const TABLE_RULE = /^\s*\|?\s*:?-{2,}:?\s*(\|\s*:?-{2,}:?\s*)*\|?\s*$/;

export function parseBlocks(src: string): Block[] {
  const lines = src.replace(/\r\n/g, '\n').split('\n');
  const blocks: Block[] = [];
  let i = 0;
  let para: string[] = [];
  const flush = () => {
    if (para.length) {
      blocks.push({ kind: 'para', text: para.join('\n') });
      para = [];
    }
  };
  while (i < lines.length) {
    const line = lines[i];
    const fence = line.match(/^\s*(```|~~~)\s*([\w+#.-]*)\s*$/);
    if (fence) {
      flush();
      const marker = fence[1];
      const body: string[] = [];
      i++;
      while (i < lines.length && !lines[i].trim().startsWith(marker)) {
        body.push(lines[i]);
        i++;
      }
      const closed = i < lines.length;
      i++; // closing fence (or end of input while streaming)
      blocks.push({ kind: 'code', lang: fence[2] ?? '', text: body.join('\n'), closed });
      continue;
    }
    // A table: a header row, a |---|---| rule, then rows.
    if (line.includes('|') && i + 1 < lines.length && lines[i + 1].includes('|') && TABLE_RULE.test(lines[i + 1])) {
      flush();
      const head = cells(line);
      const rows: string[][] = [];
      i += 2;
      while (i < lines.length && lines[i].includes('|') && lines[i].trim() !== '') {
        const r = cells(lines[i]);
        rows.push(head.map((_, c) => r[c] ?? ''));
        i++;
      }
      blocks.push({ kind: 'table', head, rows });
      continue;
    }
    const heading = line.match(/^(#{1,6})\s+(.*)$/);
    if (heading) {
      flush();
      blocks.push({ kind: 'heading', level: heading[1].length, text: heading[2] });
      i++;
      continue;
    }
    if (/^\s*([-*_])(\s*\1){2,}\s*$/.test(line)) {
      flush();
      blocks.push({ kind: 'rule' });
      i++;
      continue;
    }
    const bullet = line.match(/^\s*[-*+]\s+(.*)$/);
    const numbered = line.match(/^\s*(\d+)[.)]\s+(.*)$/);
    if (bullet || numbered) {
      flush();
      const ordered = !!numbered;
      const start = numbered ? parseInt(numbered[1], 10) : 1;
      const items: string[] = [];
      const indent = (l: string) => l.length - l.trimStart().length;
      const base = indent(line);
      while (i < lines.length) {
        const b = lines[i].match(/^\s*[-*+]\s+(.*)$/);
        const n = lines[i].match(/^\s*\d+[.)]\s+(.*)$/);
        const m = ordered ? n : b;
        if (items.length && (b || n) && indent(lines[i]) >= base + 2) {
          // Nested item: kept as a "- " line of its parent.
          items[items.length - 1] += `\n- ${(b ?? n)?.[1] ?? ''}`;
          i++;
        } else if (m && indent(lines[i]) <= base + 1) {
          items.push(m[1]);
          i++;
        } else if (items.length && /^\s{2,}\S/.test(lines[i])) {
          // Continuation or a nested list item; kept as lines of the item.
          items[items.length - 1] += `\n${lines[i].trim()}`;
          i++;
        } else if (
          items.length &&
          lines[i].trim() === '' &&
          i + 1 < lines.length &&
          /^\s{2,}[-*+\d]/.test(lines[i + 1])
        ) {
          i++;
        } else {
          break;
        }
      }
      blocks.push({ kind: 'list', ordered, items, start });
      continue;
    }
    if (/^\s*>/.test(line)) {
      flush();
      const body: string[] = [];
      while (i < lines.length && /^\s*>/.test(lines[i])) {
        body.push(lines[i].replace(/^\s*>\s?/, ''));
        i++;
      }
      blocks.push({ kind: 'quote', text: body.join('\n') });
      continue;
    }
    if (line.trim() === '') {
      flush();
    } else {
      para.push(line);
    }
    i++;
  }
  flush();
  return blocks;
}

const INLINE =
  /(`+)([\s\S]*?)\1|\*\*([\s\S]+?)\*\*|__([\s\S]+?)__|\*(?!\s)([^*\n]+?)\*|\b_(?!\s)([^_\n]+?)_\b|\[([^\]]+)\]\(([^)\s]+)\)|(https?:\/\/[^\s<>()\]]+[^\s<>()\].,;:!?'"])/g;

function renderInline(text: string, keyPrefix: string): ReactNode[] {
  const out: ReactNode[] = [];
  let last = 0;
  let k = 0;
  for (const m of text.matchAll(INLINE)) {
    const idx = m.index ?? 0;
    if (idx > last) out.push(text.slice(last, idx));
    const key = `${keyPrefix}-${k++}`;
    if (m[1]) {
      out.push(
        <Text key={key} style={styles.inlineCode}>
          {m[2]}
        </Text>,
      );
    } else if (m[3] || m[4]) {
      out.push(
        <Text key={key} style={styles.bold}>
          {renderInline(m[3] ?? m[4], key)}
        </Text>,
      );
    } else if (m[5] || m[6]) {
      out.push(
        <Text key={key} style={styles.italic}>
          {m[5] ?? m[6]}
        </Text>,
      );
    } else if (m[7]) {
      const url = m[8];
      out.push(
        <Text
          key={key}
          style={styles.link}
          onPress={() => {
            if (/^https?:\/\//i.test(url)) Linking.openURL(url).catch(() => undefined);
          }}
        >
          {m[7]}
        </Text>,
      );
    } else if (m[9]) {
      const url = m[9];
      out.push(
        <Text key={key} style={styles.link} onPress={() => Linking.openURL(url).catch(() => undefined)}>
          {url.replace(/^https?:\/\/(www\.)?/i, '').replace(/\/$/, '')}
        </Text>,
      );
    }
    last = idx + m[0].length;
  }
  if (last < text.length) out.push(text.slice(last));
  return out;
}

export function CodeBlock({ lang, text }: { lang: string; text: string }) {
  return (
    <View style={styles.codeBlock}>
      {lang ? <Text style={styles.codeLang}>{lang}</Text> : null}
      <ScrollView horizontal showsHorizontalScrollIndicator nestedScrollEnabled>
        <Text selectable style={styles.codeText}>
          {text}
        </Text>
      </ScrollView>
    </View>
  );
}

/** A link on a line of its own, like "Buy on Amazon", shown as a button. */
function LinkButton({ label, url }: { label: string; url: string }) {
  let host = '';
  try {
    host = new URL(url).hostname.replace(/^www\./, '');
  } catch {
    host = '';
  }
  return (
    <Pressable
      onPress={() => Linking.openURL(url).catch(() => undefined)}
      style={({ pressed }) => [styles.linkButton, pressed && { backgroundColor: colors.surfaceRaised }]}
      accessibilityRole="link"
      accessibilityHint={host}
    >
      <Icon name="open-in-new" size={16} color={colors.accent} />
      <Text style={styles.linkButtonText}>{label}</Text>
    </Pressable>
  );
}

const HEADING_SIZE = [24, 21, 19, 17];

const SUB = /^(?:[-*+]|\d+[.)])\s+(.*)$/;

/** A list item; nested "- sub" lines become an indented list. */
function ListItem({ text, k, color }: { text: string; k: string; color: string }) {
  const lines = text.split('\n');
  const first = lines.findIndex((l) => SUB.test(l));
  if (first < 0) {
    return (
      <Text selectable style={[styles.para, styles.listText, { color }]}>
        {renderInline(text, k)}
      </Text>
    );
  }
  const lead = lines.slice(0, first).join('\n');
  const subs: string[] = [];
  for (const l of lines.slice(first)) {
    const m = l.match(SUB);
    if (m) subs.push(m[1]);
    else if (subs.length) subs[subs.length - 1] += `\n${l}`;
  }
  return (
    <View style={styles.listText}>
      {lead ? (
        <Text selectable style={[styles.para, { color }]}>
          {renderInline(lead, k)}
        </Text>
      ) : null}
      {subs.map((sub, j) => (
        <View key={`${k}-s${j}`} style={styles.listItem}>
          <Text style={[styles.para, styles.listMarker, { color: colors.textMuted }]}>◦</Text>
          <Text selectable style={[styles.para, styles.listText, { color }]}>
            {renderInline(sub, `${k}-s${j}`)}
          </Text>
        </View>
      ))}
    </View>
  );
}

/** A table that scrolls sideways when it's wider than the screen. */
function Table({ head, rows, k, color }: { head: string[]; rows: string[][]; k: string; color: string }) {
  // Size each column to its longest cell, so small tables fit a phone.
  const plain = (t: string) => t.replace(/\[([^\]]+)\]\([^)]*\)/g, '$1').replace(/[*_`]/g, '');
  const widths = head.map((h, c) => {
    const longest = Math.max(plain(h).length, ...rows.map((r) => plain(r[c] ?? '').length));
    return Math.min(240, Math.max(72, longest * 7.5 + 24));
  });
  const cell = (t: string, key: string, bold: boolean, c: number) => (
    <View key={key} style={[styles.cell, { width: widths[c] }]}>
      <Text selectable style={[styles.cellText, { color }, bold && styles.bold]}>
        {renderInline(t, key)}
      </Text>
    </View>
  );
  return (
    <ScrollView horizontal showsHorizontalScrollIndicator nestedScrollEnabled style={styles.tableScroll}>
      <View style={styles.table}>
        <View style={[styles.tr, styles.thead]}>{head.map((h, c) => cell(h, `${k}-h${c}`, true, c))}</View>
        {rows.map((r, ri) => (
          <View key={`${k}-r${ri}`} style={[styles.tr, ri > 0 && styles.trBorder]}>
            {r.map((t, c) => cell(t, `${k}-r${ri}c${c}`, false, c))}
          </View>
        ))}
      </View>
    </ScrollView>
  );
}

function MarkdownImpl({ text, color = colors.text }: { text: string; color?: string }) {
  const blocks = useMemo(() => parseBlocks(text), [text]);
  return (
    <View>
      {blocks.map((b, i) => {
        const key = `b${i}`;
        switch (b.kind) {
          case 'code':
            return isMockup(b.lang, b.text) ? (
              <MockupCard key={key} lang={b.lang} source={b.text} complete={b.closed} />
            ) : (
              <CodeBlock key={key} lang={b.lang} text={b.text} />
            );
          case 'heading':
            return (
              <Text
                key={key}
                selectable
                style={[styles.heading, { color, fontSize: HEADING_SIZE[Math.min(b.level, 4) - 1] }]}
              >
                {renderInline(b.text, key)}
              </Text>
            );
          case 'rule':
            return <View key={key} style={styles.rule} />;
          case 'quote':
            return (
              <View key={key} style={styles.quote}>
                <Text selectable style={[styles.para, { color: colors.textMuted }]}>
                  {renderInline(b.text, key)}
                </Text>
              </View>
            );
          case 'list':
            return (
              <View key={key} style={styles.list}>
                {b.items.map((item, j) => (
                  <View key={`${key}-${j}`} style={styles.listItem}>
                    <Text style={[styles.para, styles.listMarker, { color: colors.textMuted }]}>
                      {b.ordered ? `${b.start + j}.` : '•'}
                    </Text>
                    <ListItem text={item} k={`${key}-${j}`} color={color} />
                  </View>
                ))}
              </View>
            );
          case 'table':
            return <Table key={key} head={b.head} rows={b.rows} k={key} color={color} />;
          case 'para': {
            const only = b.text.trim().match(/^\[([^\]]+)\]\((https?:\/\/[^)\s]+)\)$/i);
            if (only) return <LinkButton key={key} label={only[1]} url={only[2]} />;
            return (
              <Text key={key} selectable style={[styles.para, { color }]}>
                {renderInline(b.text, key)}
              </Text>
            );
          }
        }
      })}
    </View>
  );
}

export const Markdown = memo(MarkdownImpl);

const monoFont = Platform.OS === 'ios' ? 'Menlo' : 'monospace';

const styles = StyleSheet.create({
  para: { fontSize: 16, lineHeight: 25, marginVertical: 4, letterSpacing: 0.15 },
  heading: { fontWeight: '700', marginTop: space.md, marginBottom: space.xs },
  bold: { fontWeight: '700' },
  italic: { fontStyle: 'italic' },
  link: { color: colors.accent, textDecorationLine: 'underline' },
  inlineCode: {
    fontFamily: monoFont,
    fontSize: 14,
    backgroundColor: colors.surfaceRaised,
    color: colors.accent,
    borderRadius: 4,
  },
  codeBlock: {
    backgroundColor: colors.code,
    borderRadius: radius.md,
    padding: space.md,
    marginVertical: space.sm,
  },
  codeLang: { color: colors.textFaint, fontSize: 11, marginBottom: space.xs, textTransform: 'uppercase' },
  codeText: { fontFamily: monoFont, fontSize: 13, lineHeight: 19, color: colors.text },
  quote: { borderLeftWidth: 3, borderLeftColor: colors.border, paddingLeft: space.md, marginVertical: space.xs },
  rule: { height: StyleSheet.hairlineWidth, backgroundColor: colors.border, marginVertical: space.md },
  list: { marginVertical: 3 },
  listItem: { flexDirection: 'row' },
  listMarker: { width: 22 },
  listText: { flex: 1 },
  tableScroll: { marginVertical: space.sm },
  table: {
    borderWidth: StyleSheet.hairlineWidth,
    borderColor: colors.border,
    borderRadius: radius.md,
    overflow: 'hidden',
  },
  tr: { flexDirection: 'row' },
  trBorder: { borderTopWidth: StyleSheet.hairlineWidth, borderTopColor: colors.border },
  thead: { backgroundColor: colors.surfaceRaised },
  cell: { paddingHorizontal: space.md, paddingVertical: space.sm },
  linkButton: {
    flexDirection: 'row',
    alignItems: 'center',
    alignSelf: 'flex-start',
    gap: space.xs,
    marginVertical: space.xs,
    paddingHorizontal: space.md,
    paddingVertical: space.sm,
    borderRadius: radius.pill,
    borderWidth: 1,
    borderColor: colors.border,
  },
  linkButtonText: { color: colors.accent, fontSize: 14, fontWeight: '600' },
  cellText: { fontSize: 14, lineHeight: 20 },
});
