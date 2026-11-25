/**
 * Full-screen 3D view of a .glb file from a project, opened from a
 * "View in 3D" button in chat (the PCB skill links its board model).
 */
import { Stack, useLocalSearchParams } from 'expo-router';
import { useEffect, useState } from 'react';
import { ActivityIndicator, StyleSheet, Text, View } from 'react-native';

import ModelViewer3D from '../components/ModelViewer3D';
import { m3, space, type } from '../components/theme';
import { Banner } from '../components/ui';
import { describeError } from '../lib/api';
import { useClient } from '../lib/connection';
import { fetchProjectFile, type FileChunks } from '../lib/download';

export default function Model3DScreen() {
  const { projectId, path } = useLocalSearchParams<{ projectId: string; path: string }>();
  const client = useClient();
  const [file, setFile] = useState<FileChunks | null>(null);
  const [error, setError] = useState<string | null>(null);
  const name = (path ?? '').split('/').pop() ?? '3D view';

  useEffect(() => {
    if (!client || !projectId || !path) return;
    let live = true;
    let got: FileChunks | null = null;
    setError(null);
    fetchProjectFile(client, projectId, path)
      .then((f) => {
        got = f;
        if (live) setFile(f);
        else f.dispose();
      })
      .catch((e) => live && setError(describeError(e)));
    return () => {
      live = false;
      got?.dispose();
    };
  }, [client, projectId, path]);

  return (
    <View style={styles.flex}>
      <Stack.Screen options={{ title: name }} />
      {error ? (
        <View style={styles.pad}>
          <Banner message={`Couldn't load ${name}: ${error}`} />
        </View>
      ) : !client ? (
        <View style={styles.pad}>
          <Banner kind="info" message="Connect to your Relay server to view this model." />
        </View>
      ) : !file ? (
        <View style={styles.center}>
          <ActivityIndicator color={m3.primary} />
          <Text style={[type.bodyMedium, styles.muted]}>Downloading {name}…</Text>
        </View>
      ) : file.chunks === 0 ? (
        <View style={styles.pad}>
          <Banner message={`${name} is empty.`} />
        </View>
      ) : (
        <ModelViewer3D
          dom={{ style: styles.flex, scrollEnabled: false, bounces: false, overScrollMode: 'never' }}
          chunks={file.chunks}
          readChunk={file.read}
          background={m3.surface}
          accent={m3.primary}
          text={m3.onSurface}
          onError={(msg) => setError(msg)}
        />
      )}
    </View>
  );
}

const styles = StyleSheet.create({
  flex: { flex: 1 },
  pad: { padding: space.lg },
  center: { flex: 1, alignItems: 'center', justifyContent: 'center', gap: space.md },
  muted: { color: m3.onSurfaceVariant },
});
