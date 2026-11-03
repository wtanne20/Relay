import { Platform } from 'react-native';

import type { SessionStatus } from '../lib/types';

/**
 * Material 3 dark color scheme built from an emerald seed: green-tinted
 * near-black surfaces with tonal elevation, a soft green primary, and
 * high-contrast text. Change the seed colours here and the whole app (phone
 * and desktop) follows.
 */
export const m3 = {
  primary: '#8DD7A8',
  onPrimary: '#00391F',
  primaryContainer: '#105233',
  onPrimaryContainer: '#A9F4C3',
  secondary: '#B4CCBA',
  secondaryContainer: '#36493C',
  onSecondaryContainer: '#D0E8D6',
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

  surfaceDim: '#0F1311',
  surface: '#111412',
  surfaceContainerLowest: '#0B0F0D',
  surfaceContainerLow: '#191D1A',
  surfaceContainer: '#1D211E',
  surfaceContainerHigh: '#272B28',
  surfaceContainerHighest: '#323633',
  onSurface: '#E0E4DF',
  onSurfaceVariant: '#BFC9C0',
  outline: '#89938B',
  outlineVariant: '#3F4942',
  inverseSurface: '#E0E4DF',
  inverseOnSurface: '#2E322F',
  scrim: 'rgba(0,0,0,0.6)',
  code: '#0C100E',
  /** State layers (onSurface at M3 opacities). */
  hover: 'rgba(224,228,223,0.06)',
  pressed: 'rgba(224,228,223,0.10)',
  selected: 'rgba(141,215,168,0.12)',
  disabled: 'rgba(224,228,223,0.12)',
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
  accentPressed: '#76C493',
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
  labelLarge: { fontSize: 14, lineHeight: 20, fontWeight: '500' as const, letterSpacing: 0.1 },
  labelMedium: { fontSize: 12, lineHeight: 16, fontWeight: '500' as const, letterSpacing: 0.5 },
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
