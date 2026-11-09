import { createElement, useEffect, useState } from 'react';
import {
  ActivityIndicator,
  Image,
  Modal,
  Platform,
  Pressable,
  ScrollView,
  StyleSheet,
  Text,
  useWindowDimensions,
  View,
} from 'react-native';

import { describeError } from '../lib/api';
import { useConnection } from '../lib/connection';
import { colors, m3, mono, radius, space, type } from './theme';
import { Icon, IconButton } from './ui';

/** Blocks scripts and every network request; inline styles and data: images only. */
const CSP = "default-src 'none'; style-src 'unsafe-inline'; img-src data:; font-src data:";
/** Before the model's own styles, as the server does: no body margin, borders inside widths. */
const RESET = '<style>html,body{margin:0}*,*::before,*::after{box-sizing:border-box}</style>';
const DEFAULT_SIZE = { width: 1280, height: 800 };
/** Tall mockups (phone screens) are shrunk to this height in the chat. */
const INLINE_MAX_HEIGHT = 520;

/** HTML and SVG code blocks are drawn as mockups. */
export function isMockup(lang: string, text: string): boolean {
  const l = lang.toLowerCase();
  if (l === 'html' || l === 'svg') return true;
  return (l === '' || l === 'xml') && /^\s*<svg[\s>]/i.test(text);
}

const isSvg = (lang: string, text: string) => lang.toLowerCase() === 'svg' || /^\s*<svg[\s>]/i.test(text);

/** The `relay-size` hint (e.g. 390x844 for a phone screen), else a desktop window. */
export function mockupSize(source: string): { width: number; height: number } {
  const m = source.match(/relay-size["']?\s+content=["']?(\d+)x(\d+)/i);
  if (!m) return DEFAULT_SIZE;
  const clamp = (n: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, n));
  return { width: clamp(Number(m[1]), 240, 2560), height: clamp(Number(m[2]), 240, 4000) };
}

/** The page to show: the content policy first in <head>, as the server does. */
export function mockupPage(source: string, svg: boolean): string {
  const meta = `<meta http-equiv="Content-Security-Policy" content="${CSP}"><meta charset="utf-8">${RESET}`;
  if (svg) {
    return `<!doctype html><html><head>${meta}<style>html,body{margin:0;background:#fff}svg{display:block;max-width:100%;height:auto}</style></head><body>${source}</body></html>`;
  }
  const head = source.match(/<head(?=[\s>])[^>]*>/i);
  if (head?.index !== undefined) {
    const at = head.index + head[0].length;
    return source.slice(0, at) + meta + source.slice(at);
  }
  const html = source.match(/<html(?=[\s>])[^>]*>/i);
  if (html?.index !== undefined) {
    const at = html.index + html[0].length;
    return `${source.slice(0, at)}<head>${meta}</head>${source.slice(at)}`;
  }
  return `<!doctype html><html><head>${meta}</head><body>${source}</body></html>`;
}

/**
 * Live, sandboxed preview for the desktop app: no scripts, no network.
 *
 * Once it has loaded, the frame is measured: a design wider than the size it
 * asked for (a fixed-width layout, a drawn phone bezel) is shrunk to fit
 * whole instead of being cut off on the right. Reading the frame needs
 * allow-same-origin; scripts stay off (no allow-scripts, and the content
 * policy blocks them), so nothing inside can use that.
 */
function Frame({
  page,
  size,
  width,
  maxHeight,
}: {
  page: string;
  size: { width: number; height: number };
  width: number;
  maxHeight: number;
}) {
  const [content, setContent] = useState(size);
  useEffect(() => setContent(size), [page, size.width, size.height]); // eslint-disable-line react-hooks/exhaustive-deps
  const measure = (e: { currentTarget: HTMLIFrameElement }) => {
    const doc = e.currentTarget.contentDocument?.documentElement;
    if (!doc) return;
    const w = Math.min(4 * size.width, Math.max(size.width, Math.ceil(doc.scrollWidth)));
    if (w !== content.width) setContent({ width: w, height: Math.round((size.height * w) / size.width) });
  };
  const scale = Math.min(1, width / content.width, maxHeight / content.height);
  return (
    <View style={{ width: content.width * scale, height: content.height * scale, overflow: 'hidden' }}>
      {createElement('iframe', {
        srcDoc: page,
        sandbox: 'allow-same-origin',
        referrerPolicy: 'no-referrer',
        title: 'Mockup',
        onLoad: measure,
        style: {
          width: content.width,
          height: content.height,
          border: 0,
          // The wrapper is a flex box: without this the frame shrinks to fit
          // before it is scaled, and the mockup comes out cut short.
          flexShrink: 0,
          background: '#fff',
          transform: `scale(${scale})`,
          transformOrigin: '0 0',
        },
      })}
    </View>
  );
}

type Rendered = { uri: string; width: number; height: number };
/** Pictures already drawn this session, by source. */
const rendered = new Map<string, Rendered>();

/** The phone shows a picture the PC drew. */
function Picture({
  source,
  svg,
  width,
  maxHeight,
}: {
  source: string;
  svg: boolean;
  width: number;
  maxHeight: number;
}) {
  const { client } = useConnection();
  const key = `${svg ? 's' : 'h'}:${source}`;
  const [pic, setPic] = useState<Rendered | null>(rendered.get(key) ?? null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (pic || !client) return;
    let alive = true;
    client
      .mockup(source, svg)
      .then((r) => {
        const p = { uri: client.mockupUrl(r.id), width: r.width, height: r.height };
        rendered.set(key, p);
        if (alive) setPic(p);
      })
      .catch((e) => alive && setError(describeError(e)));
    return () => {
      alive = false;
    };
  }, [client, key, pic, source, svg]);

  if (error) {
    return (
      <View style={styles.placeholder}>
        <Icon name="image-broken-variant" size={22} color={m3.error} />
        <Text style={[type.bodySmall, { color: m3.error, textAlign: 'center' }]}>{error}</Text>
      </View>
    );
  }
  if (!pic || !client) {
    return (
      <View style={styles.placeholder}>
        <ActivityIndicator color={m3.primary} />
        <Text style={type.bodySmall}>Drawing the mockup on your PC…</Text>
      </View>
    );
  }
  const scale = Math.min(1, width / pic.width, maxHeight / pic.height);
  return (
    <Image
      source={{ uri: pic.uri, headers: { Authorization: `Bearer ${client.creds.token}` } }}
      style={{ width: pic.width * scale, height: pic.height * scale, backgroundColor: '#fff' }}
      resizeMode="contain"
      accessibilityLabel="Mockup"
    />
  );
}

function Preview(props: { source: string; svg: boolean; width: number; maxHeight: number }) {
  const { source, svg, width, maxHeight } = props;
  if (Platform.OS === 'web') {
    return <Frame page={mockupPage(source, svg)} size={mockupSize(source)} width={width} maxHeight={maxHeight} />;
  }
  return <Picture {...props} />;
}

/**
 * A mockup the model drew, as a picture card with "Code" and "Full size"
 * buttons. While the block is still streaming it shows progress instead.
 */
export function MockupCard({ lang, source, complete }: { lang: string; source: string; complete: boolean }) {
  const [width, setWidth] = useState(0);
  const [code, setCode] = useState(false);
  const [full, setFull] = useState(false);
  const win = useWindowDimensions();
  const svg = isSvg(lang, source);

  if (!complete) {
    const lines = source.split('\n').length;
    return (
      <View style={[styles.card, styles.drawing]}>
        <ActivityIndicator size="small" color={m3.primary} />
        <Text style={type.bodyMedium}>Writing a mockup… {lines} lines</Text>
      </View>
    );
  }

  return (
    <View style={styles.card}>
      <View style={styles.bar}>
        <Icon name="palette-outline" size={16} color={m3.onSurfaceVariant} />
        <Text style={[type.labelLarge, styles.title]}>Mockup</Text>
        <Pressable onPress={() => setCode((c) => !c)} style={styles.barButton} accessibilityRole="button">
          <Icon name="code-tags" size={16} color={m3.onSurfaceVariant} />
          <Text style={[type.labelMedium, styles.barText]}>{code ? 'Hide code' : 'Code'}</Text>
        </Pressable>
        <Pressable onPress={() => setFull(true)} style={styles.barButton} accessibilityRole="button">
          <Icon name="arrow-expand" size={16} color={m3.onSurfaceVariant} />
          <Text style={[type.labelMedium, styles.barText]}>Full size</Text>
        </Pressable>
      </View>
      <View style={styles.stage} onLayout={(e) => setWidth(e.nativeEvent.layout.width)}>
        {width > 0 ? <Preview source={source} svg={svg} width={width} maxHeight={INLINE_MAX_HEIGHT} /> : null}
      </View>
      {code ? (
        <ScrollView horizontal style={styles.code} nestedScrollEnabled>
          <Text selectable style={styles.codeText}>
            {source}
          </Text>
        </ScrollView>
      ) : null}

      <Modal visible={full} onRequestClose={() => setFull(false)} animationType="fade" transparent>
        <View style={styles.fullWrap}>
          <View style={styles.fullBar}>
            <Text style={[type.titleMedium, styles.grow]}>Mockup</Text>
            <IconButton icon="close" label="Close" onPress={() => setFull(false)} />
          </View>
          <ScrollView contentContainerStyle={styles.fullBody}>
            <Preview
              source={source}
              svg={svg}
              width={Math.min(win.width - 32, mockupSize(source).width)}
              maxHeight={100000}
            />
          </ScrollView>
        </View>
      </Modal>
    </View>
  );
}

const styles = StyleSheet.create({
  grow: { flex: 1 },
  card: {
    marginVertical: space.sm,
    borderRadius: radius.lg,
    borderWidth: 1,
    borderColor: m3.outlineVariant,
    backgroundColor: m3.surfaceContainerLow,
    overflow: 'hidden',
  },
  drawing: { flexDirection: 'row', alignItems: 'center', gap: space.sm, padding: space.md },
  bar: {
    flexDirection: 'row',
    alignItems: 'center',
    gap: space.sm,
    paddingLeft: space.md,
    paddingRight: space.xs,
    minHeight: 40,
  },
  title: { flex: 1, color: m3.onSurfaceVariant },
  barButton: {
    flexDirection: 'row',
    alignItems: 'center',
    gap: 4,
    paddingHorizontal: space.sm,
    height: 32,
    borderRadius: radius.sm,
  },
  barText: { color: m3.onSurfaceVariant },
  stage: { backgroundColor: m3.surfaceContainer, alignItems: 'center', paddingVertical: space.sm },
  placeholder: {
    width: '100%',
    minHeight: 160,
    alignItems: 'center',
    justifyContent: 'center',
    gap: space.sm,
    padding: space.lg,
    backgroundColor: m3.surfaceContainer,
  },
  code: { maxHeight: 320, backgroundColor: colors.code },
  codeText: { color: m3.onSurface, fontFamily: mono, fontSize: 12, lineHeight: 18, padding: space.md },
  fullWrap: { flex: 1, backgroundColor: m3.surface },
  fullBar: {
    flexDirection: 'row',
    alignItems: 'center',
    paddingLeft: space.lg,
    paddingTop: space.lg,
    paddingRight: space.sm,
  },
  fullBody: { alignItems: 'center', padding: space.md },
});
