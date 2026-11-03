import { useEffect, useState } from 'react';
import { ActivityIndicator, KeyboardAvoidingView, Platform, StyleSheet, Text, TextInput, View } from 'react-native';
import { useSafeAreaInsets } from 'react-native-safe-area-context';

import { colors, mono, radius, space } from './theme';
import { Banner, Button } from './ui';

/** A full-screen plain-text editor with Save (and optional Reset). */
export function TextEditor({
  intro,
  load,
  save,
  defaultText,
  placeholder,
}: {
  intro: string;
  load: () => Promise<string>;
  save: (text: string) => Promise<string>;
  /** Shown as "Reset to default" when given. */
  defaultText?: string;
  placeholder?: string;
}) {
  const insets = useSafeAreaInsets();
  const [original, setOriginal] = useState<string | null>(null);
  const [text, setText] = useState('');
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [savedOnce, setSavedOnce] = useState(false);

  const reload = async () => {
    try {
      const t = await load();
      setOriginal(t);
      setText(t);
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  useEffect(() => {
    reload();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const onSave = async () => {
    setSaving(true);
    try {
      const t = await save(text);
      setOriginal(t);
      setText(t);
      setSavedOnce(true);
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setSaving(false);
    }
  };

  if (original === null) {
    return (
      <View style={styles.center}>
        {error ? (
          <Banner message={error} actionLabel="Retry" onAction={reload} />
        ) : (
          <ActivityIndicator color={colors.accent} />
        )}
      </View>
    );
  }

  const dirty = text !== original;
  return (
    <KeyboardAvoidingView
      style={styles.flex}
      behavior={Platform.OS === 'ios' ? 'padding' : undefined}
      keyboardVerticalOffset={90}
    >
      <View style={[styles.flex, styles.content, { paddingBottom: Math.max(insets.bottom, space.md) }]}>
        <Text style={styles.intro}>{intro}</Text>
        {error ? <Banner message={error} /> : null}
        <TextInput
          value={text}
          onChangeText={setText}
          multiline
          placeholder={placeholder}
          placeholderTextColor={colors.textFaint}
          autoCapitalize="sentences"
          textAlignVertical="top"
          style={styles.editor}
          accessibilityLabel="Text"
        />
        <View style={styles.row}>
          {defaultText !== undefined && text !== defaultText ? (
            <Button
              title="Reset to default"
              variant="secondary"
              onPress={() => setText(defaultText)}
              style={styles.grow}
            />
          ) : null}
          <Button
            title={dirty ? 'Save' : savedOnce ? 'Saved ✓' : 'Save'}
            onPress={onSave}
            loading={saving}
            disabled={!dirty}
            style={styles.grow}
          />
        </View>
      </View>
    </KeyboardAvoidingView>
  );
}

const styles = StyleSheet.create({
  flex: { flex: 1, backgroundColor: colors.bg },
  grow: { flex: 1 },
  center: { flex: 1, backgroundColor: colors.bg, justifyContent: 'center', padding: space.lg },
  content: { padding: space.lg, gap: space.md },
  intro: { color: colors.textMuted, fontSize: 14, lineHeight: 20 },
  editor: {
    flex: 1,
    minHeight: 200,
    backgroundColor: colors.surface,
    borderColor: colors.border,
    borderWidth: 1,
    borderRadius: radius.md,
    color: colors.text,
    fontFamily: Platform.OS === 'ios' ? mono : 'monospace',
    fontSize: 14,
    lineHeight: 20,
    padding: space.md,
  },
  row: { flexDirection: 'row', gap: space.md },
});
