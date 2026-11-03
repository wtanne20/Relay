import type { SessionStatus } from '../lib/types';

export const colors = {
  bg: '#0d1117',
  surface: '#161b22',
  surfaceRaised: '#1f2630',
  border: '#30363d',
  text: '#e6edf3',
  textMuted: '#8b949e',
  textFaint: '#6e7681',
  accent: '#58a6ff',
  accentPressed: '#388bfd',
  onAccent: '#0d1117',
  success: '#3fb950',
  warning: '#d29922',
  danger: '#f85149',
  dangerBg: '#3d1a1d',
  warningBg: '#3b2e12',
  userBubble: '#1f3a5f',
  code: '#0a0e14',
};

export const space = { xs: 4, sm: 8, md: 12, lg: 16, xl: 24 };
export const radius = { sm: 6, md: 10, lg: 16, pill: 999 };
/** Minimum touch target height. */
export const touch = 48;
export const mono = 'Menlo';

export const statusColor: Record<SessionStatus, string> = {
  idle: colors.textMuted,
  running: colors.accent,
  done: colors.success,
  error: colors.danger,
  stopped: colors.warning,
};

export const statusLabel: Record<SessionStatus, string> = {
  idle: 'Idle',
  running: 'Running',
  done: 'Done',
  error: 'Error',
  stopped: 'Stopped',
};
