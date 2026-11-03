import { useEffect, useState } from 'react';
import { StyleSheet, Text, View } from 'react-native';

import { describeError } from '../lib/api';
import { useConnection } from '../lib/connection';
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
 * New project. By default it takes just a name and creates a folder for it in
 * the projects folder (asked for once, then saved in the settings). "Use an
 * existing folder" adds a folder that's already on the PC instead.
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
  const [rootDraft, setRootDraft] = useState('');
  const [existing, setExisting] = useState(false);
  const [name, setName] = useState('');
  const [path, setPath] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!visible || !client) return;
    setError(null);
    client
      .settings()
      .then((r) => setRoot(r.settings.projects_folder ?? ''))
      .catch(() => setRoot(''));
  }, [visible, client]);

  const needsRoot = !existing && root === '';
  const ready = existing ? !!path.trim() : !!name.trim() && (!needsRoot || !!rootDraft.trim());

  const submit = async () => {
    if (!ready || !client) return;
    setBusy(true);
    setError(null);
    try {
      if (needsRoot) {
        const r = await client.patchSettings({ projects_folder: rootDraft.trim() });
        setRoot(r.settings.projects_folder);
      }
      await onCreate(name.trim(), existing ? path.trim() : '');
      setName('');
      setPath('');
      setExisting(false);
    } catch (e) {
      setError(describeError(e));
    } finally {
      setBusy(false);
    }
  };

  const effectiveRoot = needsRoot ? rootDraft.trim() : (root ?? '');

  return (
    <Sheet visible={visible} onClose={onClose} title="New project">
      {existing ? (
        <>
          <Field
            label="Folder on your PC"
            icon="folder-outline"
            placeholder="C:\Users\me\code\my-app"
            value={path}
            onChangeText={setPath}
            hint="The agent can only read and change files inside this folder."
            autoFocus
          />
          <Field
            label="Name (optional)"
            icon="tag-outline"
            placeholder="Defaults to the folder name"
            value={name}
            onChangeText={setName}
            onSubmitEditing={submit}
          />
        </>
      ) : (
        <>
          {needsRoot ? (
            <Field
              label="Projects folder"
              icon="folder-home-outline"
              placeholder="C:\Users\me\Projects"
              value={rootDraft}
              onChangeText={setRootDraft}
              hint="New projects get their own folder in here. You only set this once (change it in Settings)."
              autoFocus
            />
          ) : null}
          <Field
            label="Project name"
            icon="tag-outline"
            placeholder="Mileage tracker"
            value={name}
            onChangeText={setName}
            autoFocus={!needsRoot}
            autoCapitalize="sentences"
            onSubmitEditing={submit}
          />
          {effectiveRoot && name.trim() ? (
            <Text style={[type.bodySmall, styles.preview]} numberOfLines={2}>
              Creates {projectFolderPreview(effectiveRoot, name)}
            </Text>
          ) : null}
        </>
      )}
      {error ? <Banner message={error} /> : null}
      <View style={styles.actions}>
        <Button
          title={existing ? 'New folder instead' : 'Use an existing folder'}
          variant="text"
          onPress={() => {
            setExisting((x) => !x);
            setError(null);
          }}
          style={styles.switch}
        />
        <Button title="Cancel" variant="text" onPress={onClose} />
        <Button
          title={existing ? 'Add project' : 'Create project'}
          icon="check"
          onPress={submit}
          loading={busy}
          disabled={!ready || root === null}
        />
      </View>
    </Sheet>
  );
}

const styles = StyleSheet.create({
  preview: { color: m3.onSurfaceVariant, marginTop: -space.xs, marginBottom: space.sm, marginLeft: space.lg },
  actions: { flexDirection: 'row', alignItems: 'center', gap: space.sm, marginTop: space.sm, flexWrap: 'wrap' },
  switch: { marginRight: 'auto' },
});
