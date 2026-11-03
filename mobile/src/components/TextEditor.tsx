import { useEffect, useState } from 'react';
import { ActivityIndicator, KeyboardAvoidingView, Platform, StyleSheet, Text, TextInput, View } from 'react-native';
import { useSafeAreaInsets } from 'react-native-safe-area-context';

import { usePageStyle } from '../lib/layout';
import { m3, mono, radius, space, type } from './theme';
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
  const page = usePageStyle();
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
          <ActivityIndicator color={m3.primary} />
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
      <View style={[styles.flex, styles.content, page, { paddingBottom: Math.max(insets.bottom, space.md) }]}>
        <Text style={type.bodyMedium}>{intro}</Text>
        {error ? <Banner message={error} /> : null}
        <TextInput
          value={text}
          onChangeText={setText}
          multiline
          placeholder={placeholder}
          placeholderTextColor={m3.outline}
          selectionColor={m3.primary}
          cursorColor={m3.primary}
          autoCapitalize="sentences"
          textAlignVertical="top"
          style={styles.editor}
          accessibilityLabel="Text"
        />
        <View style={styles.row}>
          {defaultText !== undefined && text !== defaultText ? (
            <Button
              title="Reset to default"
              icon="restore"
              variant="outlined"
              onPress={() => setText(defaultText)}
              style={styles.grow}
            />
          ) : null}
          <Button
            title={dirty ? 'Save' : savedOnce ? 'Saved' : 'Save'}
            icon={!dirty && savedOnce ? 'check' : 'content-save-outline'}
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
  flex: { flex: 1, backgroundColor: m3.surface },
  grow: { flex: 1 },
  center: { flex: 1, backgroundColor: m3.surface, justifyContent: 'center', padding: space.lg },
  content: { padding: space.lg, gap: space.md },
  editor: {
    flex: 1,
    minHeight: 200,
    backgroundColor: m3.surfaceContainerLow,
    borderRadius: radius.lg,
    color: m3.onSurface,
    fontFamily: mono,
    fontSize: 14,
    lineHeight: 21,
    padding: space.lg,
  },
  row: { flexDirection: 'row', gap: space.md },
});
