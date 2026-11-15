import { useEffect, useState } from 'react';
import { ActivityIndicator, Pressable, ScrollView, StyleSheet, Text, View } from 'react-native';

import { describeError } from '../lib/api';
import { useConnection } from '../lib/connection';
import { basename, formatBytes } from '../lib/format';
import type { ModelFile } from '../lib/types';
import { TagChips } from './ModelTags';
import { Sheet } from './Sheet';
import { m3, radius, space, type } from './theme';
import { Banner, Icon } from './ui';

/** "Qwen3.8-27B-Q4_K_M" from a model file path. */
export function modelLabel(path: string | null | undefined): string {
  if (!path) return '';
  return basename(path).replace(/\.gguf$/i, '');
}

/** A small pill naming the model a chat or project uses; tap to change it. */
export function ModelChip({
  model,
  fallback,
  onPress,
  disabled,
  auto,
}: {
  /** The chosen model file, or null when following the default. */
  model: string | null | undefined;
  /** What the default resolves to right now, shown dimmed. */
  fallback?: string | null;
  onPress: () => void;
  disabled?: boolean;
  /** Model routing picks the model for each message, so show "Auto". */
  auto?: boolean;
}) {
  const routed = !!auto && !model;
  const label = routed ? 'Auto' : modelLabel(model) || modelLabel(fallback) || 'Model';
  return (
    <Pressable
      onPress={onPress}
      disabled={disabled}
      style={({ hovered, pressed }: { pressed: boolean; hovered?: boolean }) => [
        styles.chip,
        (hovered || pressed) && styles.chipHover,
        disabled && { opacity: 0.6 },
      ]}
      accessibilityRole="button"
      accessibilityLabel={routed ? 'Model: Auto, picked for each message. Change model' : `Model: ${label}. Change model`}
    >
      <Icon name={routed ? 'call-split' : 'chip'} size={16} color={model ? m3.primary : m3.onSurfaceVariant} />
      <Text style={[type.labelLarge, styles.chipText]} numberOfLines={1}>
        {label}
      </Text>
      <Icon name="chevron-down" size={16} color={m3.onSurfaceVariant} />
    </Pressable>
  );
}

/**
 * Pick a model for a chat or project. The first row clears the choice so it
 * follows the default again (the project's model, or whatever is loaded).
 */
export function ModelSheet({
  visible,
  onClose,
  value,
  defaultTitle,
  defaultBody,
  onPick,
}: {
  visible: boolean;
  onClose: () => void;
  value: string | null | undefined;
  defaultTitle: string;
  defaultBody: string;
  /** Called with a model path, or '' for the default. */
  onPick: (path: string) => Promise<void>;
}) {
  const { client } = useConnection();
  const [models, setModels] = useState<ModelFile[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);

  useEffect(() => {
    if (!visible || !client) return;
    setError(null);
    client
      .models()
      .then((r) => setModels(r.models))
      .catch((e) => setError(describeError(e)));
  }, [visible, client]);

  const pick = async (path: string) => {
    setBusy(path);
    setError(null);
    try {
      await onPick(path);
      onClose();
    } catch (e) {
      setError(describeError(e));
    } finally {
      setBusy(null);
    }
  };

  const row = (key: string, selected: boolean, title: string, body: React.ReactNode, onPress: () => void) => (
    <Pressable
      key={key}
      onPress={onPress}
      disabled={busy !== null}
      style={({ hovered, pressed }: { pressed: boolean; hovered?: boolean }) => [
        styles.row,
        selected && styles.rowOn,
        (hovered || pressed) && !selected && styles.chipHover,
      ]}
      accessibilityRole="radio"
      accessibilityState={{ checked: selected }}
      accessibilityLabel={title}
    >
      <View style={styles.grow}>
        <Text style={type.bodyLarge} numberOfLines={1}>
          {title}
        </Text>
        {body}
      </View>
      {busy === key ? (
        <ActivityIndicator color={m3.primary} />
      ) : selected ? (
        <Icon name="check" size={20} color={m3.primary} />
      ) : null}
    </Pressable>
  );

  return (
    <Sheet visible={visible} onClose={onClose} title="Model">
      <Text style={[type.bodyMedium, styles.intro]}>
        Relay loads one model at a time. If the one you pick isn&apos;t loaded, Relay switches when you send your next
        message. Loading a large model can take a little while.
      </Text>
      {error ? <Banner message={error} /> : null}
      <ScrollView style={styles.list}>
        {row(
          '',
          !value,
          defaultTitle,
          <Text style={type.bodySmall} numberOfLines={2}>
            {defaultBody}
          </Text>,
          () => pick(''),
        )}
        {models === null && !error ? <ActivityIndicator color={m3.primary} style={{ margin: space.lg }} /> : null}
        {models?.map((m) =>
          row(
            m.path,
            value === m.path,
            modelLabel(m.path),
            <View style={styles.meta}>
              <Text style={type.bodySmall}>{formatBytes(m.size_bytes)}</Text>
              {m.vision ? (
                <View style={styles.vision} accessibilityLabel="Can read pictures">
                  <Icon name="eye-outline" size={14} color={m3.primary} />
                  <Text style={[type.bodySmall, { color: m3.primary }]}>Vision</Text>
                </View>
              ) : null}
              <TagChips tags={m.tags ?? []} muted={!m.custom_tags} />
            </View>,
            () => pick(m.path),
          ),
        )}
      </ScrollView>
    </Sheet>
  );
}

const styles = StyleSheet.create({
  vision: { flexDirection: 'row', alignItems: 'center', gap: 3 },
  grow: { flex: 1, minWidth: 0, gap: 2 },
  chip: {
    flexDirection: 'row',
    alignItems: 'center',
    gap: 6,
    height: 32,
    maxWidth: 260,
    flexShrink: 1,
    minWidth: 0,
    paddingHorizontal: space.md,
    borderRadius: radius.sm,
  },
  chipHover: { backgroundColor: m3.hover },
  chipText: { color: m3.onSurfaceVariant, flexShrink: 1 },
  intro: { color: m3.onSurfaceVariant, marginBottom: space.md },
  list: { maxHeight: 420 },
  row: {
    flexDirection: 'row',
    alignItems: 'center',
    gap: space.md,
    minHeight: 56,
    paddingHorizontal: space.md,
    paddingVertical: space.sm,
    borderRadius: radius.md,
  },
  rowOn: { backgroundColor: m3.secondaryContainer },
  meta: { flexDirection: 'row', alignItems: 'center', gap: space.sm, flexWrap: 'wrap' },
});
