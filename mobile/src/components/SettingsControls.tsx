import type { ReactNode } from 'react';
import { useEffect, useState } from 'react';
import { Pressable, StyleSheet, Switch, Text, TextInput, View } from 'react-native';

import { colors, radius, space, touch } from './theme';

export function SectionTitle({ title, action }: { title: string; action?: { label: string; onPress: () => void } }) {
  return (
    <View style={styles.sectionRow}>
      <Text style={styles.section}>{title}</Text>
      {action ? (
        <Pressable
          onPress={action.onPress}
          hitSlop={8}
          accessibilityRole="button"
          accessibilityLabel={`${action.label} ${title}`}
        >
          <Text style={styles.sectionAction}>{action.label}</Text>
        </Pressable>
      ) : null}
    </View>
  );
}

/** A rounded group of rows separated by hairlines. */
export function Group({ children }: { children: ReactNode }) {
  return <View style={styles.group}>{children}</View>;
}

function Label({ label, hint }: { label: string; hint?: string }) {
  return (
    <View style={styles.grow}>
      <Text style={styles.label}>{label}</Text>
      {hint ? <Text style={styles.hint}>{hint}</Text> : null}
    </View>
  );
}

export function ToggleRow({
  label,
  hint,
  value,
  onChange,
  disabled,
}: {
  label: string;
  hint?: string;
  value: boolean;
  onChange: (v: boolean) => void;
  disabled?: boolean;
}) {
  return (
    <View style={styles.row}>
      <Label label={label} hint={hint} />
      <Switch
        value={value}
        onValueChange={onChange}
        disabled={disabled}
        trackColor={{ true: colors.accent, false: colors.border }}
        accessibilityLabel={label}
      />
    </View>
  );
}

export function NavRow({ label, value, onPress }: { label: string; value?: string; onPress: () => void }) {
  return (
    <Pressable
      onPress={onPress}
      style={({ pressed }) => [styles.row, pressed && styles.pressed]}
      accessibilityRole="button"
      accessibilityLabel={label}
    >
      <Label label={label} />
      {value ? (
        <Text style={styles.navValue} numberOfLines={1} ellipsizeMode="middle">
          {value}
        </Text>
      ) : null}
      <Text style={styles.chevron}>›</Text>
    </Pressable>
  );
}

/**
 * A number input that keeps the typed text while it is being edited and
 * reports a parsed value (or null while the text is not a valid number in
 * range).
 */
export function NumberRow({
  label,
  hint,
  value,
  onChange,
  min,
  max,
  integer,
  onValidity,
}: {
  label: string;
  hint?: string;
  value: number;
  onChange: (v: number) => void;
  min: number;
  max: number;
  integer?: boolean;
  onValidity?: (valid: boolean) => void;
}) {
  const [text, setText] = useState(String(value));
  const [invalid, setInvalid] = useState(false);

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
      <Label
        label={label}
        hint={invalid ? `Enter ${integer ? 'a whole number' : 'a number'} from ${min} to ${max}` : hint}
      />
      <TextInput
        value={text}
        onChangeText={onText}
        keyboardType={integer ? 'number-pad' : 'decimal-pad'}
        style={[styles.number, invalid && styles.numberInvalid]}
        selectTextOnFocus
        accessibilityLabel={label}
        returnKeyType="done"
      />
    </View>
  );
}

export function Segmented<T extends string>({
  label,
  hint,
  value,
  options,
  onChange,
}: {
  label: string;
  hint?: string;
  value: T;
  options: { value: T; label: string }[];
  onChange: (v: T) => void;
}) {
  return (
    <View style={styles.block}>
      <Label label={label} hint={hint} />
      <View style={styles.segments} accessibilityRole="radiogroup">
        {options.map((o) => {
          const on = o.value === value;
          return (
            <Pressable
              key={o.value}
              onPress={() => onChange(o.value)}
              style={[styles.segment, on && styles.segmentOn]}
              accessibilityRole="radio"
              accessibilityState={{ selected: on }}
              accessibilityLabel={`${label}: ${o.label}`}
            >
              <Text style={[styles.segmentText, on && styles.segmentTextOn]} numberOfLines={1}>
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
  value,
  onChange,
  placeholder,
}: {
  label: string;
  hint?: string;
  value: string;
  onChange: (v: string) => void;
  placeholder?: string;
}) {
  return (
    <View style={styles.block}>
      <Label label={label} hint={hint} />
      <TextInput
        value={value}
        onChangeText={onChange}
        placeholder={placeholder}
        placeholderTextColor={colors.textFaint}
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
    alignItems: 'flex-end',
    marginTop: space.xl,
    marginBottom: space.sm,
  },
  section: { color: colors.textMuted, fontSize: 13, fontWeight: '600', textTransform: 'uppercase' },
  sectionAction: { color: colors.accent, fontSize: 14, fontWeight: '600' },
  group: {
    backgroundColor: colors.surface,
    borderRadius: radius.md,
    borderWidth: StyleSheet.hairlineWidth,
    borderColor: colors.border,
    overflow: 'hidden',
  },
  row: {
    flexDirection: 'row',
    alignItems: 'center',
    gap: space.md,
    minHeight: touch + 8,
    paddingHorizontal: space.lg,
    paddingVertical: space.sm,
    borderBottomWidth: StyleSheet.hairlineWidth,
    borderBottomColor: colors.border,
  },
  block: {
    gap: space.sm,
    paddingHorizontal: space.lg,
    paddingVertical: space.md,
    borderBottomWidth: StyleSheet.hairlineWidth,
    borderBottomColor: colors.border,
  },
  pressed: { backgroundColor: colors.surfaceRaised },
  label: { color: colors.text, fontSize: 16, fontWeight: '500' },
  hint: { color: colors.textMuted, fontSize: 13, marginTop: 2, lineHeight: 18 },
  navValue: { color: colors.textMuted, fontSize: 15, maxWidth: '50%' },
  chevron: { color: colors.textFaint, fontSize: 24, marginTop: -2 },
  number: {
    width: 112,
    flexShrink: 0,
    height: 40,
    borderRadius: radius.sm,
    borderWidth: 1,
    borderColor: colors.border,
    backgroundColor: colors.bg,
    color: colors.text,
    fontSize: 16,
    textAlign: 'right',
    paddingHorizontal: space.md,
  },
  numberInvalid: { borderColor: colors.danger },
  text: {
    height: 44,
    borderRadius: radius.sm,
    borderWidth: 1,
    borderColor: colors.border,
    backgroundColor: colors.bg,
    color: colors.text,
    fontSize: 15,
    paddingHorizontal: space.md,
  },
  segments: {
    flexDirection: 'row',
    backgroundColor: colors.bg,
    borderRadius: radius.sm,
    borderWidth: 1,
    borderColor: colors.border,
    padding: 3,
    gap: 3,
  },
  segment: { flex: 1, minHeight: 40, borderRadius: radius.sm - 2, alignItems: 'center', justifyContent: 'center' },
  segmentOn: { backgroundColor: colors.accent },
  segmentText: { color: colors.textMuted, fontSize: 14, fontWeight: '600' },
  segmentTextOn: { color: colors.onAccent },
});
