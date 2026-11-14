import { router } from 'expo-router';
import { useEffect, useMemo, useState } from 'react';
import { ActivityIndicator, StyleSheet, Text, View } from 'react-native';

import { describeError } from '../lib/api';
import { useConnection } from '../lib/connection';
import { useWide } from '../lib/layout';
import { ComputerPicker, FolderBrowser, OfflineBanner, useComputers, useFolderBrowser } from './FolderPicker';
import { useBrainInfo, useHere } from './Location';
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
 * New project. On the brain: just a name, and Relay creates a folder with
 * that name inside the projects folder set in Settings. On a linked
 * computer: pick one of its folders.
 */
export function AddProjectSheet({
  visible,
  onClose,
  onCreate,
}: {
  visible: boolean;
  onClose: () => void;
  onCreate: (name: string, path: string, deviceId?: string) => Promise<void>;
}) {
  const { client } = useConnection();
  const brain = useBrainInfo();
  const here = useHere();
  const [root, setRoot] = useState<string | null>(null);
  const [name, setName] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const { devices, where, setWhere, device } = useComputers(visible);
  // On the brain a new project gets its own folder, so there is nothing to browse.
  const load = useMemo(
    () => (client && where ? (path: string) => client.browseDevice(where, path) : null),
    [client, where],
  );
  const { folder, browsing, browse, clear } = useFolderBrowser(load, setError);

  useEffect(() => {
    if (!visible || !client) return;
    setError(null);
    client
      .settings()
      .then((r) => setRoot(r.settings.projects_folder ?? ''))
      .catch((e) => setError(describeError(e)));
  }, [visible, client]);

  useEffect(() => {
    clear();
    if (visible && where) browse('');
  }, [visible, where, browse, clear]);

  const submit = async () => {
    if (where) {
      if (!folder) return;
      setBusy(true);
      setError(null);
      try {
        await onCreate(name.trim(), folder.path, where);
        setName('');
      } catch (e) {
        setError(describeError(e));
      } finally {
        setBusy(false);
      }
      return;
    }
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

  const brainName = brain?.hostname || 'the brain';

  return (
    <Sheet visible={visible} onClose={onClose} title="New project">
      <ComputerPicker label="Where the files are" devices={devices} value={where} onChange={setWhere} />
      {where ? (
        <>
          <OfflineBanner device={device} />
          <FolderBrowser folder={folder} browsing={browsing} onBrowse={browse} />
          <Field
            label="Project name (optional)"
            icon="tag-outline"
            placeholder={folder ? folder.path.split(/[\\/]/).filter(Boolean).pop() ?? '' : ''}
            value={name}
            onChangeText={setName}
            autoCapitalize="sentences"
            onSubmitEditing={submit}
          />
          {error ? <Banner message={error} /> : null}
          <View style={styles.actions}>
            <Button title="Cancel" variant="text" onPress={onClose} />
            <Button title="Use this folder" icon="check" onPress={submit} loading={busy} disabled={!folder} />
          </View>
        </>
      ) : root === null && !error ? (
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
              Creates {projectFolderPreview(root, name)} on {here.isBrain ? 'this computer' : brainName}
            </Text>
          ) : null}
          {error ? <Banner message={error} /> : null}
          <View style={styles.actions}>
            <Button title="Cancel" variant="text" onPress={onClose} />
            <Button title="Create" icon="check" onPress={submit} loading={busy} disabled={!name.trim()} />
          </View>
        </>
      )}
      {!where && root === null && error ? <Banner message={error} /> : null}
    </Sheet>
  );
}

const styles = StyleSheet.create({
  loading: { marginVertical: space.xl },
  text: { color: m3.onSurfaceVariant, marginBottom: space.md },
  preview: { color: m3.onSurfaceVariant, marginTop: -space.xs, marginBottom: space.sm, marginLeft: space.lg },
  actions: { flexDirection: 'row', justifyContent: 'flex-end', gap: space.sm, marginTop: space.sm },
});
