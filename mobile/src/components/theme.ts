import * as Secure from 'expo-secure-store';
import { Platform, StyleSheet } from 'react-native';

import type { SessionStatus } from '../lib/types';

/** Accent colours the user can pick (Settings, Appearance). Tones follow M3 dark. */
export const ACCENTS = {
  green: {
    name: 'Green',
    primary: '#8DD7A8',
    onPrimary: '#00391F',
    container: '#105233',
    onContainer: '#A9F4C3',
    pressed: '#76C493',
  },
  blue: {
    name: 'Blue',
    primary: '#A8C7FA',
    onPrimary: '#062E6F',
    container: '#0842A0',
    onContainer: '#D3E3FD',
    pressed: '#8AB4F8',
  },
  purple: {
    name: 'Purple',
    primary: '#CFBCFF',
    onPrimary: '#381E72',
    container: '#4F378B',
    onContainer: '#E9DDFF',
    pressed: '#B69DF8',
  },
  orange: {
    name: 'Orange',
    primary: '#FFB77C',
    onPrimary: '#4E2600',
    container: '#6F3800',
    onContainer: '#FFDCC2',
    pressed: '#F59E5B',
  },
  pink: {
    name: 'Pink',
    primary: '#FFB0CD',
    onPrimary: '#5E1133',
    container: '#7B2949',
    onContainer: '#FFD9E2',
    pressed: '#F48FB4',
  },
  teal: {
    name: 'Teal',
    primary: '#80D5D0',
    onPrimary: '#003735',
    container: '#00504D',
    onContainer: '#9CF2EC',
    pressed: '#66BDB8',
  },
} as const;
export type AccentId = keyof typeof ACCENTS;

const ACCENT_KEY = 'relay.accent';

function storedAccent(): AccentId {
  try {
    const v = Platform.OS === 'web' ? globalThis.localStorage?.getItem(ACCENT_KEY) : Secure.getItem(ACCENT_KEY);
    if (v && v in ACCENTS) return v as AccentId;
  } catch {
    // No stored choice yet, or storage unavailable: use the default.
  }
  return 'green';
}

function rgba(hex: string, alpha: number): string {
  const n = parseInt(hex.slice(1), 16);
  return `rgba(${(n >> 16) & 255},${(n >> 8) & 255},${n & 255},${alpha})`;
}

/** The accent-coloured tokens for one accent. */
function accentTokens(id: AccentId) {
  const a = ACCENTS[id];
  return {
    primary: a.primary,
    onPrimary: a.onPrimary,
    primaryContainer: a.container,
    onPrimaryContainer: a.onContainer,
    primaryPressed: a.pressed,
    primaryBorder: rgba(a.primary, 0.35),
    selected: rgba(a.primary, 0.12),
  };
}

/** The accent the app started with; styles created at startup use it. */
const bootAccent = storedAccent();
let currentAccent: AccentId = bootAccent;

/**
 * Material 3 dark color scheme: neutral grey surfaces with tonal elevation,
 * and the user's accent colour (green by default) only for accents: primary
 * actions, highlights, focus and status.
 */
export const m3 = {
  ...accentTokens(bootAccent),
  secondary: '#C6C6C6',
  secondaryContainer: '#383838',
  onSecondaryContainer: '#EDEDED',
  tertiary: '#A3CDDB',
  tertiaryContainer: '#214C58',
  onTertiaryContainer: '#BFE9F8',
  error: '#FFB4AB',
  onError: '#690005',
  errorContainer: '#5C1A14',
  onErrorContainer: '#FFDAD6',
  success: '#9DDC9C',
  successContainer: '#1C3A22',
  warning: '#F0C46C',
  warningContainer: '#3D2F10',
  onWarningContainer: '#FFE0A3',

  surfaceDim: '#111111',
  surface: '#141414',
  surfaceContainerLowest: '#0E0E0E',
  surfaceContainerLow: '#1B1B1B',
  surfaceContainer: '#202020',
  surfaceContainerHigh: '#2A2A2A',
  surfaceContainerHighest: '#353535',
  onSurface: '#E6E6E6',
  onSurfaceVariant: '#C2C2C2',
  outline: '#8E8E8E',
  outlineVariant: '#424242',
  inverseSurface: '#E6E6E6',
  inverseOnSurface: '#303030',
  scrim: 'rgba(0,0,0,0.6)',
  code: '#0F0F0F',
  /** State layers (onSurface at M3 opacities). */
  hover: 'rgba(230,230,230,0.06)',
  pressed: 'rgba(230,230,230,0.10)',
  disabled: 'rgba(230,230,230,0.12)',
};

/** Older names used across the app, mapped onto the M3 roles. */
export const colors = {
  bg: m3.surface,
  surface: m3.surfaceContainer,
  surfaceRaised: m3.surfaceContainerHigh,
  border: m3.outlineVariant,
  text: m3.onSurface,
  textMuted: m3.onSurfaceVariant,
  textFaint: m3.outline,
  accent: m3.primary,
  accentPressed: m3.primaryPressed,
  onAccent: m3.onPrimary,
  success: m3.success,
  warning: m3.warning,
  danger: m3.error,
  dangerBg: m3.errorContainer,
  warningBg: m3.warningContainer,
  userBubble: m3.surfaceContainerHighest,
  code: m3.code,
};

export const space = { xs: 4, sm: 8, md: 12, lg: 16, xl: 24, xxl: 32 };
/** M3 shape scale. */
export const radius = { xs: 4, sm: 8, md: 12, lg: 16, xl: 28, pill: 999 };
/** Minimum touch target. */
export const touch = 48;

export const mono = Platform.select({ ios: 'Menlo', android: 'monospace', default: 'monospace' }) as string;
/** Serif display face for greetings, like Claude's app. */
export const serif = Platform.select({ ios: 'Georgia', android: 'serif', default: 'Georgia, serif' }) as string;

/** M3 type scale (system font: Roboto on Android, SF on iOS). */
export const type = {
  displaySmall: { fontSize: 34, lineHeight: 42, fontFamily: serif, color: m3.onSurface },
  headlineMedium: { fontSize: 28, lineHeight: 36, fontFamily: serif, color: m3.onSurface },
  headlineSmall: { fontSize: 24, lineHeight: 32, color: m3.onSurface },
  titleLarge: { fontSize: 22, lineHeight: 28, color: m3.onSurface },
  titleMedium: { fontSize: 16, lineHeight: 24, fontWeight: '500' as const, letterSpacing: 0.15, color: m3.onSurface },
  titleSmall: { fontSize: 14, lineHeight: 20, fontWeight: '500' as const, letterSpacing: 0.1, color: m3.onSurface },
  bodyLarge: { fontSize: 16, lineHeight: 24, letterSpacing: 0.15, color: m3.onSurface },
  bodyMedium: { fontSize: 14, lineHeight: 20, letterSpacing: 0.25, color: m3.onSurfaceVariant },
  bodySmall: { fontSize: 12, lineHeight: 16, letterSpacing: 0.4, color: m3.onSurfaceVariant },
  labelLarge: { fontSize: 14, lineHeight: 20, fontWeight: '500' as const, letterSpacing: 0.1, color: m3.onSurface },
  labelMedium: { fontSize: 12, lineHeight: 16, fontWeight: '500' as const, letterSpacing: 0.5, color: m3.onSurface },
};

export const statusColor: Record<SessionStatus, string> = {
  idle: m3.onSurfaceVariant,
  running: m3.primary,
  done: m3.success,
  error: m3.error,
  stopped: m3.warning,
};

export const statusContainer: Record<SessionStatus, string> = {
  idle: m3.surfaceContainerHighest,
  running: m3.primaryContainer,
  done: m3.successContainer,
  error: m3.errorContainer,
  stopped: m3.warningContainer,
};

export const statusLabel: Record<SessionStatus, string> = {
  idle: 'Idle',
  running: 'Running',
  done: 'Done',
  error: 'Error',
  stopped: 'Stopped',
};

// ---------- switching the accent while the app runs ----------

/**
 * Styles are made once, at startup, with the accent of that moment. To switch
 * accents without restarting, every StyleSheet hands out its styles through
 * getters that swap the startup accent's colours for the current ones.
 */
let swap: Map<string, string> | null = null;
const caches = new Set<Map<string, unknown>>();

function recolor(v: unknown): unknown {
  if (!swap) return v;
  if (typeof v === 'string') return swap.get(v.toLowerCase()) ?? v;
  if (Array.isArray(v)) return v.map(recolor);
  if (v && typeof v === 'object') {
    const out: Record<string, unknown> = {};
    for (const [k, x] of Object.entries(v)) out[k] = recolor(x);
    return out;
  }
  return v;
}

if (Platform.OS !== 'web') {
  // The web app reloads instead (see setAccent).
  const create = StyleSheet.create.bind(StyleSheet);
  (StyleSheet as { create: typeof StyleSheet.create }).create = ((styles: Record<string, unknown>) => {
    const sheet = create(styles as never) as Record<string, unknown>;
    const cache = new Map<string, unknown>();
    caches.add(cache);
    const out: Record<string, unknown> = {};
    for (const key of Object.keys(sheet)) {
      Object.defineProperty(out, key, {
        enumerable: true,
        get() {
          if (!swap) return sheet[key];
          if (!cache.has(key)) cache.set(key, recolor(sheet[key]));
          return cache.get(key);
        },
      });
    }
    return out;
  }) as typeof StyleSheet.create;
}

const listeners = new Set<() => void>();

export function accent(): AccentId {
  return currentAccent;
}

export function onAccentChange(f: () => void): () => void {
  listeners.add(f);
  return () => {
    listeners.delete(f);
  };
}

/** Saves the accent and recolours the app. */
export function setAccent(id: AccentId) {
  if (id === currentAccent) return;
  try {
    if (Platform.OS === 'web') globalThis.localStorage?.setItem(ACCENT_KEY, id);
    else Secure.setItem(ACCENT_KEY, id);
  } catch {
    // Still switch for this session.
  }
  if (Platform.OS === 'web') {
    // The browser and desktop app simply reload with the new colours.
    globalThis.location?.reload();
    return;
  }
  const from = accentTokens(currentAccent);
  const to = accentTokens(id);
  const boot = accentTokens(bootAccent);
  swap = new Map(Object.entries(boot).map(([k, v]) => [v.toLowerCase(), to[k as keyof typeof to]]));
  caches.forEach((c) => c.clear());
  // Plain objects made from the tokens are updated in place.
  Object.assign(m3, to);
  const re = (o: Record<string, string>) => {
    for (const [k, v] of Object.entries(o)) {
      const hit = Object.entries(from).find(([, fv]) => fv.toLowerCase() === v.toLowerCase());
      if (hit) o[k] = to[hit[0] as keyof typeof to];
    }
  };
  re(colors);
  re(statusColor);
  re(statusContainer);
  currentAccent = id;
  listeners.forEach((f) => f());
}
