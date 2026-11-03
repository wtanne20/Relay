import { router } from 'expo-router';
import { useEffect, useState } from 'react';
import { ActivityIndicator, StyleSheet, Text, View } from 'react-native';

import { describeError } from '../lib/api';
import { useConnection } from '../lib/connection';
import { useWide } from '../lib/layout';
import { Sheet } from './Sheet';
import { m3, space, type } from './theme';
import { Banner, Button, Field } from './ui';

/** Where a new project named `name` will live inside `root`. */
export function projectFolderPreview(root: string, name: string): string {
  const sep = root.includes('\\') ? '\\' : '/';
  const clean = name
    .replace(/[<>:"/\\|?*]/g, '-')
    .trim()
    .replace(/[. ]+$/, '');
  return `${root.replace(/[\\/]+$/, '')}${sep}${clean || '…'}`;
}

/**
 * New project: just a name. Relay creates a folder with that name inside the
 * projects folder set in Settings.
 */
export function AddProjectSheet({
  visible,
  onClose,
  onCreate,
}: {
  visible: boolean;
  onClose: () => void;
  onCreate: (name: string, path: string) => Promise<void>;
}) {
  const { client } = useConnection();
  const [root, setRoot] = useState<string | null>(null);
  const [name, setName] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!visible || !client) return;
    setError(null);
    client
      .settings()
      .then((r) => setRoot(r.settings.projects_folder ?? ''))
      .catch((e) => setError(describeError(e)));
  }, [visible, client]);

  const submit = async () => {
    if (!name.trim() || !root) return;
    setBusy(true);
    setError(null);
    try {
      await onCreate(name.trim(), '');
      setName('');
    } catch (e) {
      setError(describeError(e));
    } finally {
      setBusy(false);
    }
  };

  const wide = useWide();
  const openSettings = () => {
    onClose();
    // Desktop swaps the main pane; on the phone it's a normal screen you go back from.
    if (wide) router.replace('/settings');
    else router.push('/settings');
  };

  return (
    <Sheet visible={visible} onClose={onClose} title="New project">
      {root === null && !error ? (
        <ActivityIndicator color={m3.primary} style={styles.loading} />
      ) : root === '' ? (
        <>
          <Text style={[type.bodyMedium, styles.text]}>
            First choose the folder your projects live in. Each new project gets its own folder inside it.
          </Text>
          <View style={styles.actions}>
            <Button title="Cancel" variant="text" onPress={onClose} />
            <Button title="Open Settings" icon="cog-outline" onPress={openSettings} />
          </View>
        </>
      ) : (
        <>
          <Field
            label="Project name"
            icon="tag-outline"
            placeholder="Mileage tracker"
            value={name}
            onChangeText={setName}
            autoFocus
            autoCapitalize="sentences"
            onSubmitEditing={submit}
          />
          {root ? (
            <Text style={[type.bodySmall, styles.preview]} numberOfLines={2}>
              Creates {projectFolderPreview(root, name)}
            </Text>
          ) : null}
          {error ? <Banner message={error} /> : null}
          <View style={styles.actions}>
            <Button title="Cancel" variant="text" onPress={onClose} />
            <Button title="Create" icon="check" onPress={submit} loading={busy} disabled={!name.trim()} />
          </View>
        </>
      )}
      {root === null && error ? <Banner message={error} /> : null}
    </Sheet>
  );
}

const styles = StyleSheet.create({
  loading: { marginVertical: space.xl },
  text: { color: m3.onSurfaceVariant, marginBottom: space.md },
  preview: { color: m3.onSurfaceVariant, marginTop: -space.xs, marginBottom: space.sm, marginLeft: space.lg },
  actions: { flexDirection: 'row', justifyContent: 'flex-end', gap: space.sm, marginTop: space.sm },
});
