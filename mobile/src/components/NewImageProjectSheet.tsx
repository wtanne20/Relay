import { useEffect, useState } from 'react';
import { StyleSheet, Text, View } from 'react-native';

import { describeError } from '../lib/api';
import { useClient } from '../lib/connection';
import { imagesApi, type ImageProject } from '../lib/images';
import { Sheet } from './Sheet';
import { m3, space, type } from './theme';
import { Banner, Button, Field } from './ui';

/** Names a new image project (a group of image chats that share a style). */
export function NewImageProjectSheet({
  visible,
  onClose,
  onCreated,
}: {
  visible: boolean;
  onClose: () => void;
  onCreated: (p: ImageProject) => void;
}) {
  const client = useClient();
  const [name, setName] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    if (visible) {
      setName('');
      setError(null);
    }
  }, [visible]);

  const create = async () => {
    if (!client || !name.trim()) return;
    setBusy(true);
    try {
      onCreated(await imagesApi(client).createProject(name.trim()));
    } catch (e) {
      setError(describeError(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Sheet visible={visible} onClose={onClose} title="New image project">
      <Text style={[type.bodyMedium, styles.hint]}>
        A project holds image chats. Each chat develops one picture, message by message.
      </Text>
      <Field
        label="Name"
        value={name}
        onChangeText={setName}
        placeholder="Logo ideas"
        autoCapitalize="sentences"
        autoFocus
        onSubmitEditing={create}
      />
      {error ? <Banner message={error} /> : null}
      <View style={styles.actions}>
        <Button title="Cancel" variant="text" onPress={onClose} />
        <Button title="Create" onPress={create} loading={busy} disabled={!name.trim()} />
      </View>
    </Sheet>
  );
}

const styles = StyleSheet.create({
  hint: { color: m3.onSurfaceVariant, marginBottom: space.md },
  actions: { flexDirection: 'row', justifyContent: 'flex-end', gap: space.sm, marginTop: space.sm },
});
