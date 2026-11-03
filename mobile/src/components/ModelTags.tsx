import { useEffect, useState } from 'react';
import { Pressable, StyleSheet, Text, View } from 'react-native';

import { describeError } from '../lib/api';
import { Sheet } from './Sheet';
import { m3, radius, space, type } from './theme';
import { Banner, Button, Field, Icon } from './ui';

export const PRESET_TAGS = ['code', 'UI design', 'web', 'apps', 'reasoning', 'chat', 'writing', 'vision', 'fast'];

/** Small read-only tag chips. */
export function TagChips({ tags, muted }: { tags: string[]; muted?: boolean }) {
  if (!tags.length) return null;
  return (
    <View style={styles.chips}>
      {tags.map((t) => (
        <View key={t} style={[styles.chip, muted && styles.chipMuted]}>
          <Text style={[type.labelMedium, { color: muted ? m3.onSurfaceVariant : m3.primary }]}>{t}</Text>
        </View>
      ))}
    </View>
  );
}

/** Edit a model's tags: toggle common ones or add your own. */
export function TagsSheet({
  model,
  onClose,
  onSave,
}: {
  model: { name: string; tags: string[]; custom: boolean } | null;
  onClose: () => void;
  onSave: (tags: string[] | null) => Promise<void>;
}) {
  const [tags, setTags] = useState<string[]>([]);
  const [custom, setCustom] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    setTags(model?.tags ?? []);
    setCustom('');
    setError(null);
  }, [model]);

  const toggle = (t: string) =>
    setTags((cur) =>
      cur.some((x) => x.toLowerCase() === t.toLowerCase())
        ? cur.filter((x) => x.toLowerCase() !== t.toLowerCase())
        : [...cur, t],
    );
  const addCustom = () => {
    const t = custom.trim();
    if (t && !tags.some((x) => x.toLowerCase() === t.toLowerCase())) setTags([...tags, t]);
    setCustom('');
  };
  const save = async (value: string[] | null) => {
    setBusy(true);
    setError(null);
    try {
      await onSave(value);
    } catch (e) {
      setError(describeError(e));
    } finally {
      setBusy(false);
    }
  };

  const options = [
    ...PRESET_TAGS,
    ...tags.filter((t) => !PRESET_TAGS.some((p) => p.toLowerCase() === t.toLowerCase())),
  ];

  return (
    <Sheet visible={model !== null} onClose={onClose} title="Model tags">
      <Text style={[type.bodyMedium, styles.name]} numberOfLines={1} ellipsizeMode="middle">
        {model?.name}
      </Text>
      <View style={[styles.chips, styles.options]}>
        {options.map((t) => {
          const on = tags.some((x) => x.toLowerCase() === t.toLowerCase());
          return (
            <Pressable
              key={t}
              onPress={() => toggle(t)}
              style={[styles.option, on && styles.optionOn]}
              accessibilityRole="checkbox"
              accessibilityState={{ checked: on }}
              accessibilityLabel={t}
            >
              {on ? <Icon name="check" size={16} color={m3.onSecondaryContainer} /> : null}
              <Text style={[type.labelLarge, { color: on ? m3.onSecondaryContainer : m3.onSurfaceVariant }]}>{t}</Text>
            </Pressable>
          );
        })}
      </View>
      <Field
        label="Add your own"
        icon="tag-plus-outline"
        placeholder="e.g. React"
        value={custom}
        onChangeText={setCustom}
        onSubmitEditing={addCustom}
        returnKeyType="done"
      />
      {error ? <Banner message={error} /> : null}
      <View style={styles.actions}>
        {model?.custom ? (
          <Button title="Use automatic tags" variant="text" onPress={() => save(null)} style={styles.reset} />
        ) : null}
        <Button title="Cancel" variant="text" onPress={onClose} />
        <Button
          title="Save"
          icon="check"
          loading={busy}
          onPress={() => {
            const extra = custom.trim();
            save(extra && !tags.some((x) => x.toLowerCase() === extra.toLowerCase()) ? [...tags, extra] : tags);
          }}
        />
      </View>
    </Sheet>
  );
}

const styles = StyleSheet.create({
  chips: { flexDirection: 'row', flexWrap: 'wrap', gap: 6 },
  chip: {
    borderRadius: radius.sm,
    borderWidth: 1,
    borderColor: 'rgba(141,215,168,0.35)',
    paddingHorizontal: 8,
    paddingVertical: 1,
  },
  chipMuted: { borderColor: m3.outlineVariant },
  name: { marginBottom: space.md },
  options: { marginBottom: space.lg, gap: space.sm },
  option: {
    flexDirection: 'row',
    alignItems: 'center',
    gap: 4,
    height: 32,
    paddingHorizontal: space.md,
    borderRadius: radius.sm,
    borderWidth: 1,
    borderColor: m3.outline,
  },
  optionOn: { backgroundColor: m3.secondaryContainer, borderColor: m3.secondaryContainer },
  actions: {
    flexDirection: 'row',
    alignItems: 'center',
    justifyContent: 'flex-end',
    gap: space.sm,
    marginTop: space.sm,
    flexWrap: 'wrap',
  },
  reset: { marginRight: 'auto' },
});
