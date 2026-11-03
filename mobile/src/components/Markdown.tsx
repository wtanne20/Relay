import { memo, useMemo, type ReactNode } from 'react';
import { Linking, Platform, ScrollView, StyleSheet, Text, View } from 'react-native';

import { colors, radius, space } from './theme';

/**
 * A small Markdown renderer covering what coding agents produce: fenced code
 * blocks, headings, bullet/numbered lists, block quotes, paragraphs, and
 * inline `code`, **bold**, *italic* and [links](url). Streaming-safe: an
 * unterminated code fence renders as code.
 */

type Block =
  | { kind: 'code'; lang: string; text: string }
  | { kind: 'heading'; level: number; text: string }
  | { kind: 'list'; ordered: boolean; items: string[]; start: number }
  | { kind: 'quote'; text: string }
  | { kind: 'rule' }
  | { kind: 'para'; text: string };

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
      i++; // closing fence (or end of input while streaming)
      blocks.push({ kind: 'code', lang: fence[2] ?? '', text: body.join('\n') });
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
      while (i < lines.length) {
        const b = lines[i].match(/^\s*[-*+]\s+(.*)$/);
        const n = lines[i].match(/^\s*\d+[.)]\s+(.*)$/);
        const m = ordered ? n : b;
        if (m) {
          items.push(m[1]);
          i++;
        } else if (items.length && /^\s{2,}\S/.test(lines[i])) {
          items[items.length - 1] += `\n${lines[i].trim()}`;
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

const INLINE = /(`+)([\s\S]*?)\1|\*\*([\s\S]+?)\*\*|__([\s\S]+?)__|\*(?!\s)([^*\n]+?)\*|\b_(?!\s)([^_\n]+?)_\b|\[([^\]]+)\]\(([^)\s]+)\)/g;

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
    }
    last = idx + m[0].length;
  }
  if (last < text.length) out.push(text.slice(last));
  return out;
}

function CodeBlock({ lang, text }: { lang: string; text: string }) {
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

function MarkdownImpl({ text, color = colors.text }: { text: string; color?: string }) {
  const blocks = useMemo(() => parseBlocks(text), [text]);
  return (
    <View>
      {blocks.map((b, i) => {
        const key = `b${i}`;
        switch (b.kind) {
          case 'code':
            return <CodeBlock key={key} lang={b.lang} text={b.text} />;
          case 'heading':
            return (
              <Text key={key} selectable style={[styles.heading, { color, fontSize: Math.max(16, 22 - b.level * 2) }]}>
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
                    <Text selectable style={[styles.para, styles.listText, { color }]}>
                      {renderInline(item, `${key}-${j}`)}
                    </Text>
                  </View>
                ))}
              </View>
            );
          case 'para':
            return (
              <Text key={key} selectable style={[styles.para, { color }]}>
                {renderInline(b.text, key)}
              </Text>
            );
        }
      })}
    </View>
  );
}

export const Markdown = memo(MarkdownImpl);

const monoFont = Platform.OS === 'ios' ? 'Menlo' : 'monospace';

const styles = StyleSheet.create({
  para: { fontSize: 16, lineHeight: 23, marginVertical: 3 },
  heading: { fontWeight: '700', marginTop: space.sm, marginBottom: space.xs },
  bold: { fontWeight: '700' },
  italic: { fontStyle: 'italic' },
  link: { color: colors.accent, textDecorationLine: 'underline' },
  inlineCode: {
    fontFamily: monoFont,
    fontSize: 14,
    backgroundColor: colors.surfaceRaised,
    color: '#ffa657',
  },
  codeBlock: {
    backgroundColor: colors.code,
    borderRadius: radius.sm,
    borderWidth: StyleSheet.hairlineWidth,
    borderColor: colors.border,
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
});
