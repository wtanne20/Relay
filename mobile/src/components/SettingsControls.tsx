import type { ReactNode } from 'react';
import { useEffect, useState } from 'react';
import { Pressable, StyleSheet, Switch, Text, TextInput, View } from 'react-native';

import { m3, radius, space, type } from './theme';
import { Icon, type IconName } from './ui';

/** M3 list section header (primary-colored label). */
export function SectionTitle({ title, action }: { title: string; action?: { label: string; onPress: () => void } }) {
  return (
    <View style={styles.sectionRow}>
      <Text style={[type.titleSmall, { color: m3.primary }]}>{title}</Text>
      {action ? (
        <Pressable
          onPress={action.onPress}
          hitSlop={8}
          style={styles.sectionAction}
          accessibilityRole="button"
          accessibilityLabel={`${action.label} ${title}`}
        >
          <Icon name="restore" size={16} color={m3.primary} />
          <Text style={[type.labelLarge, { color: m3.primary }]}>{action.label}</Text>
        </Pressable>
      ) : null}
    </View>
  );
}

/** A rounded M3 container for a group of list rows. */
export function Group({ children }: { children: ReactNode }) {
  return <View style={styles.group}>{children}</View>;
}

function Leading({ icon }: { icon?: IconName }) {
  if (!icon) return null;
  return (
    <View style={styles.leading}>
      <Icon name={icon} size={22} color={m3.onSurfaceVariant} />
    </View>
  );
}

function Label({ label, hint }: { label: string; hint?: string }) {
  return (
    <View style={styles.grow}>
      <Text style={type.bodyLarge}>{label}</Text>
      {hint ? <Text style={type.bodyMedium}>{hint}</Text> : null}
    </View>
  );
}

export function ToggleRow({
  label,
  hint,
  icon,
  value,
  onChange,
  disabled,
}: {
  label: string;
  hint?: string;
  icon?: IconName;
  value: boolean;
  onChange: (v: boolean) => void;
  disabled?: boolean;
}) {
  return (
    <Pressable
      onPress={() => !disabled && onChange(!value)}
      style={({ pressed }) => [styles.row, pressed && styles.pressed]}
      accessibilityRole="switch"
      accessibilityState={{ checked: value, disabled }}
      accessibilityLabel={label}
    >
      <Leading icon={icon} />
      <Label label={label} hint={hint} />
      <Switch
        value={value}
        onValueChange={onChange}
        disabled={disabled}
        trackColor={{ true: m3.primary, false: m3.surfaceContainerHighest }}
        thumbColor={value ? m3.onPrimary : m3.outline}
        // react-native-web ignores thumbColor for the on state
        {...({ activeThumbColor: m3.onPrimary } as object)}
        ios_backgroundColor={m3.surfaceContainerHighest}
      />
    </Pressable>
  );
}

export function NavRow({
  label,
  value,
  icon,
  onPress,
}: {
  label: string;
  value?: string;
  icon?: IconName;
  onPress: () => void;
}) {
  return (
    <Pressable
      onPress={onPress}
      style={({ pressed }) => [styles.row, pressed && styles.pressed]}
      accessibilityRole="button"
      accessibilityLabel={label}
    >
      <Leading icon={icon} />
      <View style={styles.grow}>
        <Text style={type.bodyLarge}>{label}</Text>
        {value ? (
          <Text style={type.bodyMedium} numberOfLines={1} ellipsizeMode="middle">
            {value}
          </Text>
        ) : null}
      </View>
      <Icon name="chevron-right" color={m3.outline} />
    </Pressable>
  );
}

/**
 * A number input that keeps the typed text while it is being edited and
 * reports a parsed value only when it is a valid number in range.
 */
export function NumberRow({
  label,
  hint,
  icon,
  value,
  onChange,
  min,
  max,
  integer,
  onValidity,
}: {
  label: string;
  hint?: string;
  icon?: IconName;
  value: number;
  onChange: (v: number) => void;
  min: number;
  max: number;
  integer?: boolean;
  onValidity?: (valid: boolean) => void;
}) {
  const [text, setText] = useState(String(value));
  const [invalid, setInvalid] = useState(false);
  const [focused, setFocused] = useState(false);

  useEffect(() => {
    // Follow outside changes (reset to defaults, reload).
    const parsed = Number(text.replace(',', '.'));
    if (parsed !== value) {
      setText(String(value));
      setInvalid(false);
      onValidity?.(true);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [value]);

  const onText = (t: string) => {
    setText(t);
    const n = Number(t.replace(',', '.'));
    const ok = t.trim() !== '' && Number.isFinite(n) && n >= min && n <= max && (!integer || Number.isInteger(n));
    setInvalid(!ok);
    onValidity?.(ok);
    if (ok) onChange(n);
  };

  return (
    <View style={styles.row}>
      <Leading icon={icon} />
      <View style={styles.grow}>
        <Text style={type.bodyLarge}>{label}</Text>
        {invalid ? (
          <Text style={[type.bodyMedium, { color: m3.error }]}>
            {integer ? 'Whole number' : 'Number'} from {min} to {max}
          </Text>
        ) : hint ? (
          <Text style={type.bodyMedium}>{hint}</Text>
        ) : null}
      </View>
      <TextInput
        value={text}
        onChangeText={onText}
        onFocus={() => setFocused(true)}
        onBlur={() => setFocused(false)}
        keyboardType={integer ? 'number-pad' : 'decimal-pad'}
        style={[styles.number, focused && styles.numberFocused, invalid && styles.numberInvalid]}
        selectTextOnFocus
        selectionColor={m3.primary}
        accessibilityLabel={label}
        returnKeyType="done"
      />
    </View>
  );
}

/** M3 segmented buttons. */
export function Segmented<T extends string>({
  label,
  hint,
  icon,
  value,
  options,
  onChange,
}: {
  label: string;
  hint?: string;
  icon?: IconName;
  value: T;
  options: { value: T; label: string }[];
  onChange: (v: T) => void;
}) {
  return (
    <View style={styles.block}>
      <View style={styles.blockHead}>
        <Leading icon={icon} />
        <Label label={label} hint={hint} />
      </View>
      <View style={styles.segments} accessibilityRole="radiogroup">
        {options.map((o, i) => {
          const on = o.value === value;
          return (
            <Pressable
              key={o.value}
              onPress={() => onChange(o.value)}
              style={[
                styles.segment,
                i === 0 && styles.segmentFirst,
                i === options.length - 1 && styles.segmentLast,
                i > 0 && styles.segmentJoin,
                on && styles.segmentOn,
              ]}
              accessibilityRole="radio"
              accessibilityState={{ selected: on }}
              accessibilityLabel={`${label}: ${o.label}`}
            >
              {on ? <Icon name="check" size={16} color={m3.onSecondaryContainer} /> : null}
              <Text style={[type.labelLarge, { color: on ? m3.onSecondaryContainer : m3.onSurface }]} numberOfLines={1}>
                {o.label}
              </Text>
            </Pressable>
          );
        })}
      </View>
    </View>
  );
}

export function TextRow({
  label,
  hint,
  icon,
  value,
  onChange,
  placeholder,
}: {
  label: string;
  hint?: string;
  icon?: IconName;
  value: string;
  onChange: (v: string) => void;
  placeholder?: string;
}) {
  return (
    <View style={styles.block}>
      <View style={styles.blockHead}>
        <Leading icon={icon} />
        <Label label={label} hint={hint} />
      </View>
      <TextInput
        value={value}
        onChangeText={onChange}
        placeholder={placeholder}
        placeholderTextColor={m3.outline}
        selectionColor={m3.primary}
        autoCapitalize="none"
        autoCorrect={false}
        style={styles.text}
        accessibilityLabel={label}
      />
    </View>
  );
}

const styles = StyleSheet.create({
  grow: { flex: 1, minWidth: 0 },
  sectionRow: {
    flexDirection: 'row',
    justifyContent: 'space-between',
    alignItems: 'center',
    marginTop: space.xl,
    marginBottom: space.sm,
    paddingHorizontal: space.lg,
    minHeight: 32,
  },
  sectionAction: { flexDirection: 'row', alignItems: 'center', gap: 4 },
  group: {
    backgroundColor: m3.surfaceContainerLow,
    borderRadius: radius.xl,
    overflow: 'hidden',
    paddingVertical: space.xs,
  },
  leading: { width: 24, alignItems: 'center' },
  row: {
    flexDirection: 'row',
    alignItems: 'center',
    gap: space.lg,
    minHeight: 64,
    paddingHorizontal: space.lg,
    paddingVertical: space.sm,
  },
  block: { gap: space.md, paddingHorizontal: space.lg, paddingVertical: space.md },
  blockHead: { flexDirection: 'row', alignItems: 'center', gap: space.lg },
  pressed: { backgroundColor: m3.pressed },
  number: {
    width: 104,
    flexShrink: 0,
    height: 44,
    borderRadius: radius.md,
    backgroundColor: m3.surfaceContainerHighest,
    color: m3.onSurface,
    fontSize: 16,
    textAlign: 'center',
    paddingHorizontal: space.md,
    borderWidth: 2,
    borderColor: 'transparent',
  },
  numberFocused: { borderColor: m3.primary },
  numberInvalid: { borderColor: m3.error },
  text: {
    height: 48,
    borderRadius: radius.md,
    backgroundColor: m3.surfaceContainerHighest,
    color: m3.onSurface,
    fontSize: 15,
    paddingHorizontal: space.lg,
  },
  segments: { flexDirection: 'row' },
  segment: {
    flex: 1,
    minHeight: 40,
    flexDirection: 'row',
    gap: 6,
    alignItems: 'center',
    justifyContent: 'center',
    borderWidth: 1,
    borderColor: m3.outline,
    paddingHorizontal: space.sm,
  },
  segmentFirst: { borderTopLeftRadius: 20, borderBottomLeftRadius: 20 },
  segmentLast: { borderTopRightRadius: 20, borderBottomRightRadius: 20 },
  segmentJoin: { marginLeft: -1 },
  segmentOn: { backgroundColor: m3.secondaryContainer },
});
