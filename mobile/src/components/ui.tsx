import type { ReactNode } from 'react';
import {
  ActivityIndicator,
  Platform,
  Pressable,
  StyleSheet,
  Text,
  TextInput,
  View,
  type StyleProp,
  type TextInputProps,
  type ViewStyle,
} from 'react-native';

import type { SessionStatus } from '../lib/types';
import { colors, mono, radius, space, statusColor, statusLabel, touch } from './theme';

type Variant = 'primary' | 'secondary' | 'danger' | 'ghost';

export function Button({
  title,
  onPress,
  variant = 'primary',
  disabled,
  loading,
  style,
  accessibilityLabel,
}: {
  title: string;
  onPress: () => void;
  variant?: Variant;
  disabled?: boolean;
  loading?: boolean;
  style?: StyleProp<ViewStyle>;
  accessibilityLabel?: string;
}) {
  const bg: Record<Variant, string> = {
    primary: colors.accent,
    secondary: colors.surfaceRaised,
    danger: colors.dangerBg,
    ghost: 'transparent',
  };
  const fg: Record<Variant, string> = {
    primary: colors.onAccent,
    secondary: colors.text,
    danger: colors.danger,
    ghost: colors.accent,
  };
  const off = disabled || loading;
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={accessibilityLabel ?? title}
      accessibilityState={{ disabled: !!off, busy: !!loading }}
      onPress={onPress}
      disabled={off}
      style={({ pressed }) => [
        styles.button,
        { backgroundColor: bg[variant], opacity: off ? 0.5 : pressed ? 0.75 : 1 },
        variant === 'secondary' && styles.buttonBorder,
        style,
      ]}
    >
      {loading ? (
        <ActivityIndicator color={fg[variant]} />
      ) : (
        <Text style={[styles.buttonText, { color: fg[variant] }]}>{title}</Text>
      )}
    </Pressable>
  );
}

export function Field({ label, hint, ...props }: TextInputProps & { label: string; hint?: string }) {
  return (
    <View style={styles.field}>
      <Text style={styles.label}>{label}</Text>
      <TextInput
        placeholderTextColor={colors.textFaint}
        autoCapitalize="none"
        autoCorrect={false}
        {...props}
        style={[styles.input, props.multiline && styles.inputMultiline, props.style]}
      />
      {hint ? <Text style={styles.hint}>{hint}</Text> : null}
    </View>
  );
}

export function Banner({
  kind = 'error',
  message,
  actionLabel,
  onAction,
}: {
  kind?: 'error' | 'warning' | 'info';
  message: string;
  actionLabel?: string;
  onAction?: () => void;
}) {
  const bg = kind === 'error' ? colors.dangerBg : kind === 'warning' ? colors.warningBg : colors.surfaceRaised;
  const fg = kind === 'error' ? colors.danger : kind === 'warning' ? colors.warning : colors.text;
  return (
    <View style={[styles.banner, { backgroundColor: bg }]} accessibilityRole="alert">
      <Text style={[styles.bannerText, { color: fg }]}>{message}</Text>
      {actionLabel && onAction ? (
        <Pressable onPress={onAction} style={styles.bannerAction} accessibilityRole="button" hitSlop={8}>
          <Text style={[styles.bannerActionText, { color: fg }]}>{actionLabel}</Text>
        </Pressable>
      ) : null}
    </View>
  );
}

export function StatusBadge({ status }: { status: SessionStatus }) {
  const c = statusColor[status];
  return (
    <View style={[styles.badge, { borderColor: c }]}>
      {status === 'running' ? <ActivityIndicator size="small" color={c} style={styles.badgeSpinner} /> : null}
      <Text style={[styles.badgeText, { color: c }]}>{statusLabel[status]}</Text>
    </View>
  );
}

export function Dot({ color }: { color: string }) {
  return <View style={[styles.dot, { backgroundColor: color }]} />;
}

export function Card({ children, style }: { children: ReactNode; style?: StyleProp<ViewStyle> }) {
  return <View style={[styles.card, style]}>{children}</View>;
}

export function EmptyState({ title, body }: { title: string; body?: string }) {
  return (
    <View style={styles.empty}>
      <Text style={styles.emptyTitle}>{title}</Text>
      {body ? <Text style={styles.emptyBody}>{body}</Text> : null}
    </View>
  );
}

export function Mono({ children, style }: { children: ReactNode; style?: StyleProp<ViewStyle> }) {
  return (
    <View style={style}>
      <Text selectable style={styles.mono}>
        {children}
      </Text>
    </View>
  );
}

const styles = StyleSheet.create({
  button: {
    minHeight: touch,
    paddingHorizontal: space.lg,
    borderRadius: radius.md,
    alignItems: 'center',
    justifyContent: 'center',
  },
  buttonBorder: { borderWidth: StyleSheet.hairlineWidth, borderColor: colors.border },
  buttonText: { fontSize: 16, fontWeight: '600' },
  field: { marginBottom: space.lg },
  label: { color: colors.textMuted, fontSize: 14, marginBottom: space.xs, fontWeight: '500' },
  input: {
    minHeight: touch,
    backgroundColor: colors.surface,
    borderColor: colors.border,
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: radius.md,
    color: colors.text,
    fontSize: 16,
    paddingHorizontal: space.md,
    paddingVertical: Platform.OS === 'ios' ? space.md : space.sm,
  },
  inputMultiline: { minHeight: 96, textAlignVertical: 'top' },
  hint: { color: colors.textFaint, fontSize: 13, marginTop: space.xs },
  banner: {
    flexDirection: 'row',
    alignItems: 'center',
    paddingHorizontal: space.md,
    paddingVertical: space.sm,
    borderRadius: radius.md,
    marginHorizontal: space.md,
    marginVertical: space.sm,
    gap: space.sm,
  },
  bannerText: { flex: 1, fontSize: 14 },
  bannerAction: { minHeight: 36, justifyContent: 'center', paddingHorizontal: space.sm },
  bannerActionText: { fontWeight: '700', fontSize: 14 },
  badge: {
    flexDirection: 'row',
    alignItems: 'center',
    borderWidth: 1,
    borderRadius: radius.pill,
    paddingHorizontal: space.sm,
    paddingVertical: 2,
  },
  badgeSpinner: { transform: [{ scale: 0.6 }], marginLeft: -4, marginRight: 0 },
  badgeText: { fontSize: 12, fontWeight: '600' },
  dot: { width: 10, height: 10, borderRadius: 5 },
  card: {
    backgroundColor: colors.surface,
    borderRadius: radius.lg,
    borderWidth: StyleSheet.hairlineWidth,
    borderColor: colors.border,
    padding: space.lg,
  },
  empty: { alignItems: 'center', padding: space.xl, marginTop: space.xl },
  emptyTitle: { color: colors.text, fontSize: 18, fontWeight: '600', textAlign: 'center' },
  emptyBody: { color: colors.textMuted, fontSize: 15, marginTop: space.sm, textAlign: 'center', lineHeight: 21 },
  mono: { color: colors.text, fontFamily: Platform.OS === 'ios' ? mono : 'monospace', fontSize: 13 },
});
