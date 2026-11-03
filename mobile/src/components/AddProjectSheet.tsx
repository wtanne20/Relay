import { useState } from 'react';
import { StyleSheet, View } from 'react-native';

import { describeError } from '../lib/api';
import { Sheet } from './Sheet';
import { space } from './theme';
import { Banner, Button, Field } from './ui';

/** Asks for a folder on the PC (and an optional name) to add as a project. */
export function AddProjectSheet({
  visible,
  onClose,
  onCreate,
}: {
  visible: boolean;
  onClose: () => void;
  onCreate: (name: string, path: string) => Promise<void>;
}) {
  const [name, setName] = useState('');
  const [path, setPath] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const submit = async () => {
    setBusy(true);
    setError(null);
    try {
      await onCreate(name.trim(), path.trim());
      setName('');
      setPath('');
    } catch (e) {
      setError(describeError(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Sheet visible={visible} onClose={onClose} title="New project">
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
      />
      {error ? <Banner message={error} /> : null}
      <View style={styles.sheetActions}>
        <Button title="Cancel" variant="text" onPress={onClose} />
        <Button title="Add project" icon="check" onPress={submit} loading={busy} disabled={!path.trim()} />
      </View>
    </Sheet>
  );
}

const styles = StyleSheet.create({
  sheetActions: { flexDirection: 'row', justifyContent: 'flex-end', gap: space.sm, marginTop: space.sm },
});
