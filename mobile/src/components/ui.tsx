import MaterialCommunityIcons from '@expo/vector-icons/MaterialCommunityIcons';
import type { ComponentProps, ReactNode } from 'react';
import { useState } from 'react';
import {
  ActivityIndicator,
  Image,
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
import { m3, mono, radius, space, statusColor, statusContainer, statusLabel, touch, type } from './theme';

export type IconName = ComponentProps<typeof MaterialCommunityIcons>['name'];

const LOGO = require('../../assets/logo.png');

/** The Relay brain mark on a rounded clay tile. */
export function Logo({ size = 32 }: { size?: number }) {
  return (
    <View
      style={{
        width: size,
        height: size,
        borderRadius: size * 0.3,
        backgroundColor: m3.primaryContainer,
        alignItems: 'center',
        justifyContent: 'center',
      }}
      accessibilityLabel="Relay"
    >
      <Image source={LOGO} style={{ width: size * 0.72, height: size * 0.72 }} />
    </View>
  );
}

export function Icon({
  name,
  size = 24,
  color = m3.onSurfaceVariant,
}: {
  name: IconName;
  size?: number;
  color?: string;
}) {
  return <MaterialCommunityIcons name={name} size={size} color={color} />;
}

type Variant = 'filled' | 'tonal' | 'outlined' | 'text' | 'danger' | 'primary' | 'secondary' | 'ghost';

/** M3 common button (pill). `primary`/`secondary`/`ghost` are aliases. */
export function Button({
  title,
  onPress,
  variant = 'filled',
  icon,
  disabled,
  loading,
  style,
  accessibilityLabel,
}: {
  title: string;
  onPress: () => void;
  variant?: Variant;
  icon?: IconName;
  disabled?: boolean;
  loading?: boolean;
  style?: StyleProp<ViewStyle>;
  accessibilityLabel?: string;
}) {
  const v: Variant =
    variant === 'primary' ? 'filled' : variant === 'secondary' ? 'tonal' : variant === 'ghost' ? 'text' : variant;
  const bg = {
    filled: m3.primary,
    tonal: m3.secondaryContainer,
    outlined: 'transparent',
    text: 'transparent',
    danger: m3.errorContainer,
  }[v as 'filled'];
  const fg = {
    filled: m3.onPrimary,
    tonal: m3.onSecondaryContainer,
    outlined: m3.primary,
    text: m3.primary,
    danger: m3.onErrorContainer,
  }[v as 'filled'];
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
        { backgroundColor: bg },
        v === 'outlined' && styles.buttonOutlined,
        v === 'text' && styles.buttonText,
        off && v !== 'text' && v !== 'outlined' && styles.buttonDisabled,
        pressed && styles.pressedOverlay,
        style,
      ]}
    >
      {loading ? (
        <ActivityIndicator color={fg} />
      ) : (
        <>
          {icon ? <Icon name={icon} size={18} color={off ? m3.outline : fg} /> : null}
          <Text
            style={[type.labelLarge, { color: off && v !== 'filled' ? m3.outline : off ? m3.onSurfaceVariant : fg }]}
          >
            {title}
          </Text>
        </>
      )}
    </Pressable>
  );
}

export function IconButton({
  icon,
  onPress,
  label,
  color = m3.onSurfaceVariant,
  variant = 'standard',
  size = 24,
  disabled,
}: {
  icon: IconName;
  onPress: () => void;
  label: string;
  color?: string;
  variant?: 'standard' | 'tonal' | 'filled';
  size?: number;
  disabled?: boolean;
}) {
  const bg = variant === 'filled' ? m3.primary : variant === 'tonal' ? m3.secondaryContainer : 'transparent';
  const fg = variant === 'filled' ? m3.onPrimary : variant === 'tonal' ? m3.onSecondaryContainer : color;
  return (
    <Pressable
      onPress={onPress}
      disabled={disabled}
      accessibilityRole="button"
      accessibilityLabel={label}
      hitSlop={4}
      style={({ pressed }) => [
        styles.iconButton,
        { backgroundColor: bg, opacity: disabled ? 0.38 : 1 },
        pressed && styles.iconPressed,
      ]}
    >
      <Icon name={icon} size={size} color={fg} />
    </Pressable>
  );
}

/** M3 extended floating action button. */
export function Fab({
  icon,
  label,
  onPress,
  style,
}: {
  icon: IconName;
  label: string;
  onPress: () => void;
  style?: StyleProp<ViewStyle>;
}) {
  return (
    <Pressable
      onPress={onPress}
      accessibilityRole="button"
      accessibilityLabel={label}
      style={({ pressed }) => [styles.fab, pressed && styles.pressedOverlay, style]}
    >
      <Icon name={icon} color={m3.onPrimaryContainer} />
      <Text style={[type.labelLarge, { color: m3.onPrimaryContainer, fontSize: 15 }]}>{label}</Text>
    </Pressable>
  );
}

/** M3 outlined text field with a floating-style label above. */
export function Field({
  label,
  hint,
  icon,
  ...props
}: TextInputProps & { label: string; hint?: string; icon?: IconName }) {
  const [focused, setFocused] = useState(false);
  return (
    <View style={styles.field}>
      <Text style={[type.bodySmall, styles.fieldLabel, focused && { color: m3.primary }]}>{label}</Text>
      <View style={[styles.inputWrap, focused && styles.inputFocused, props.multiline && styles.inputMultiline]}>
        {icon ? <Icon name={icon} size={20} color={focused ? m3.primary : m3.onSurfaceVariant} /> : null}
        <TextInput
          placeholderTextColor={m3.outline}
          autoCapitalize="none"
          autoCorrect={false}
          selectionColor={m3.primary}
          cursorColor={m3.primary}
          {...props}
          onFocus={(e) => {
            setFocused(true);
            props.onFocus?.(e);
          }}
          onBlur={(e) => {
            setFocused(false);
            props.onBlur?.(e);
          }}
          style={[styles.input, props.style]}
        />
      </View>
      {hint ? <Text style={[type.bodySmall, styles.hint]}>{hint}</Text> : null}
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
  const bg = kind === 'error' ? m3.errorContainer : kind === 'warning' ? m3.warningContainer : m3.surfaceContainerHigh;
  const fg = kind === 'error' ? m3.onErrorContainer : kind === 'warning' ? m3.onWarningContainer : m3.onSurface;
  const icon: IconName =
    kind === 'error' ? 'alert-circle-outline' : kind === 'warning' ? 'wifi-alert' : 'information-outline';
  return (
    <View style={[styles.banner, { backgroundColor: bg }]} accessibilityRole="alert">
      <Icon name={icon} size={20} color={fg} />
      <Text style={[type.bodyMedium, styles.bannerText, { color: fg }]}>{message}</Text>
      {actionLabel && onAction ? (
        <Pressable onPress={onAction} style={styles.bannerAction} accessibilityRole="button" hitSlop={8}>
          <Text style={[type.labelLarge, { color: fg }]}>{actionLabel}</Text>
        </Pressable>
      ) : null}
    </View>
  );
}

/** M3 assist-chip style status indicator. */
export function StatusBadge({ status }: { status: SessionStatus }) {
  const c = statusColor[status];
  const icon: IconName =
    status === 'done'
      ? 'check'
      : status === 'error'
        ? 'alert-circle-outline'
        : status === 'stopped'
          ? 'stop'
          : 'circle-small';
  return (
    <View style={[styles.badge, { backgroundColor: statusContainer[status] }]}>
      {status === 'running' ? <ActivityIndicator size={12} color={c} /> : <Icon name={icon} size={14} color={c} />}
      <Text style={[type.labelMedium, { color: c }]}>{statusLabel[status]}</Text>
    </View>
  );
}

export function Dot({ color }: { color: string }) {
  return <View style={[styles.dot, { backgroundColor: color }]} />;
}

/** M3 filled card. */
export function Card({ children, style }: { children: ReactNode; style?: StyleProp<ViewStyle> }) {
  return <View style={[styles.card, style]}>{children}</View>;
}

export function EmptyState({ title, body, icon }: { title: string; body?: string; icon?: IconName }) {
  return (
    <View style={styles.empty}>
      {icon ? (
        <View style={styles.emptyIcon}>
          <Icon name={icon} size={32} color={m3.primary} />
        </View>
      ) : null}
      <Text style={[type.headlineMedium, styles.center]}>{title}</Text>
      {body ? <Text style={[type.bodyLarge, styles.emptyBody]}>{body}</Text> : null}
    </View>
  );
}

/** A tonal rounded-square avatar with an icon or initial. */
export function Avatar({ icon, label, size = 40 }: { icon?: IconName; label?: string; size?: number }) {
  return (
    <View style={[styles.avatar, { width: size, height: size, borderRadius: size * 0.3 }]}>
      {icon ? (
        <Icon name={icon} size={size * 0.55} color={m3.onPrimaryContainer} />
      ) : (
        <Text style={[type.titleMedium, { color: m3.onPrimaryContainer }]}>
          {(label ?? '?').slice(0, 1).toUpperCase()}
        </Text>
      )}
    </View>
  );
}

/** M3 two-line list item. */
export function ListItem({
  title,
  subtitle,
  leading,
  trailing,
  onPress,
  onLongPress,
  accessibilityHint,
}: {
  title: string;
  subtitle?: string;
  leading?: ReactNode;
  trailing?: ReactNode;
  onPress?: () => void;
  onLongPress?: () => void;
  accessibilityHint?: string;
}) {
  return (
    <Pressable
      onPress={onPress}
      onLongPress={onLongPress}
      disabled={!onPress && !onLongPress}
      accessibilityRole={onPress ? 'button' : undefined}
      accessibilityLabel={title}
      accessibilityHint={accessibilityHint}
      style={({ pressed }) => [styles.listItem, pressed && styles.listPressed]}
    >
      {leading}
      <View style={styles.grow}>
        <Text style={type.bodyLarge} numberOfLines={1}>
          {title}
        </Text>
        {subtitle ? (
          <Text style={type.bodyMedium} numberOfLines={1} ellipsizeMode="middle">
            {subtitle}
          </Text>
        ) : null}
      </View>
      {trailing}
    </Pressable>
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
  grow: { flex: 1, minWidth: 0 },
  center: { textAlign: 'center' },
  button: {
    minHeight: touch,
    borderRadius: radius.pill,
    paddingHorizontal: space.xl,
    flexDirection: 'row',
    alignItems: 'center',
    justifyContent: 'center',
    gap: space.sm,
  },
  buttonOutlined: { borderWidth: 1, borderColor: m3.outline },
  buttonText: { paddingHorizontal: space.md },
  buttonDisabled: { backgroundColor: 'rgba(237,229,225,0.12)' },
  pressedOverlay: { opacity: 0.85 },
  iconButton: { width: 44, height: 44, borderRadius: 22, alignItems: 'center', justifyContent: 'center' },
  iconPressed: { backgroundColor: 'rgba(237,229,225,0.10)' },
  fab: {
    height: 56,
    borderRadius: radius.lg,
    backgroundColor: m3.primaryContainer,
    paddingHorizontal: 20,
    flexDirection: 'row',
    alignItems: 'center',
    gap: space.md,
    shadowColor: '#000',
    shadowOpacity: 0.35,
    shadowRadius: 8,
    shadowOffset: { width: 0, height: 4 },
    elevation: 6,
  },
  field: { marginBottom: space.lg },
  fieldLabel: { marginBottom: 6, marginLeft: space.xs },
  inputWrap: {
    minHeight: 56,
    flexDirection: 'row',
    alignItems: 'center',
    gap: space.md,
    borderRadius: radius.md,
    borderWidth: 1,
    borderColor: m3.outline,
    paddingHorizontal: space.lg,
    backgroundColor: m3.surfaceContainerLow,
  },
  inputFocused: { borderColor: m3.primary, borderWidth: 2, paddingHorizontal: space.lg - 1 },
  inputMultiline: { alignItems: 'flex-start', paddingVertical: space.md },
  input: { flex: 1, minWidth: 0, color: m3.onSurface, fontSize: 16, paddingVertical: space.md },
  hint: { marginTop: 6, marginLeft: space.xs },
  banner: {
    flexDirection: 'row',
    alignItems: 'center',
    gap: space.md,
    borderRadius: radius.lg,
    paddingLeft: space.lg,
    paddingRight: space.sm,
    paddingVertical: space.sm,
    marginHorizontal: space.lg,
    marginVertical: space.sm,
    minHeight: touch,
  },
  bannerText: { flex: 1, minWidth: 0 },
  bannerAction: { minHeight: 40, justifyContent: 'center', paddingHorizontal: space.md },
  badge: {
    flexDirection: 'row',
    alignItems: 'center',
    gap: 4,
    height: 28,
    borderRadius: radius.sm,
    paddingLeft: space.sm,
    paddingRight: space.md,
  },
  dot: { width: 10, height: 10, borderRadius: 5 },
  card: { backgroundColor: m3.surfaceContainer, borderRadius: radius.xl, padding: space.lg },
  empty: { alignItems: 'center', paddingHorizontal: space.xl, paddingVertical: 48, gap: space.md },
  emptyIcon: {
    width: 72,
    height: 72,
    borderRadius: 24,
    backgroundColor: m3.primaryContainer,
    alignItems: 'center',
    justifyContent: 'center',
    marginBottom: space.sm,
  },
  emptyBody: { textAlign: 'center', color: m3.onSurfaceVariant },
  avatar: { backgroundColor: m3.primaryContainer, alignItems: 'center', justifyContent: 'center' },
  listItem: {
    minHeight: 72,
    flexDirection: 'row',
    alignItems: 'center',
    gap: space.lg,
    paddingHorizontal: space.lg,
    paddingVertical: space.sm,
  },
  listPressed: { backgroundColor: 'rgba(237,229,225,0.08)' },
  mono: { color: m3.onSurface, fontFamily: mono, fontSize: 13 },
});
