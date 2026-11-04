import { ActivityIndicator, Linking, StyleSheet, Text, View } from 'react-native';

import type { RelayClient } from '../lib/api';
import { appVersion, updateKind, useUpdates } from '../lib/updates';
import { m3, radius, space, type } from './theme';
import { Button, Icon } from './ui';

function actionLabel(version: string): string {
  if (updateKind === 'desktop') return `Install ${version} and restart`;
  if (updateKind === 'android') return `Download and install ${version}`;
  return 'Open the release page';
}

/**
 * Relay's version and updates. With `onlyWhenAvailable` it shows nothing
 * until there's a new version (for the home screen).
 */
export function UpdateCard({ client, onlyWhenAvailable }: { client: RelayClient | null; onlyWhenAvailable?: boolean }) {
  const u = useUpdates(client);
  const latest = u.info?.latest;
  const busy = u.phase.kind === 'downloading' || u.phase.kind === 'installing';
  if (onlyWhenAvailable && !u.available && !busy) return null;

  const mine = updateKind === 'android' ? appVersion() : (u.info?.current ?? null);
  let status: string;
  if (u.phase.kind === 'downloading') status = `Downloading… ${Math.round(u.phase.progress * 100)}%`;
  else if (u.phase.kind === 'installing')
    status =
      updateKind === 'desktop'
        ? 'Installing. Windows may ask for permission; Relay closes and opens again by itself.'
        : 'Follow Android’s prompt to install the update.';
  else if (u.phase.kind === 'error') status = u.phase.message;
  else if (u.checking && !u.info) status = 'Checking for updates…';
  else if (u.available && latest) status = mine ? `You have ${mine}.` : 'A new version is out.';
  else if (u.info?.error) status = `Couldn't check for updates: ${u.info.error}`;
  else if (latest) status = 'You have the latest version.';
  else status = '';

  return (
    <View style={[styles.card, u.available && styles.highlight]}>
      <View style={styles.head}>
        <Icon name={u.available ? 'download-circle-outline' : 'check-circle-outline'} color={u.available ? m3.primary : m3.onSurfaceVariant} />
        <View style={styles.grow}>
          <Text style={type.titleSmall}>
            {u.available && latest ? `Relay ${latest.version} is available` : mine ? `Relay ${mine}` : 'Relay'}
          </Text>
          {status ? (
            <Text style={[type.bodyMedium, u.phase.kind === 'error' && { color: m3.error }]}>{status}</Text>
          ) : null}
        </View>
        {u.checking ? <ActivityIndicator color={m3.primary} /> : null}
      </View>
      {u.phase.kind === 'downloading' ? (
        <View style={styles.track}>
          <View style={[styles.fill, { width: `${Math.max(2, u.phase.progress * 100)}%` }]} />
        </View>
      ) : null}
      <View style={styles.actions}>
        {u.available && latest ? (
          <Button title={actionLabel(latest.version)} icon="download" onPress={u.install} disabled={busy} />
        ) : null}
        {latest?.page && u.available ? (
          <Button title="What's new" variant="text" onPress={() => Linking.openURL(latest.page)} />
        ) : null}
        {!onlyWhenAvailable ? (
          <Button title="Check now" variant="text" onPress={() => u.check(true)} disabled={busy || u.checking} />
        ) : null}
      </View>
    </View>
  );
}

const styles = StyleSheet.create({
  card: {
    padding: space.lg,
    gap: space.md,
    borderRadius: radius.lg,
    backgroundColor: m3.surfaceContainer,
  },
  highlight: { borderWidth: 1, borderColor: m3.primaryBorder },
  head: { flexDirection: 'row', alignItems: 'center', gap: space.md },
  grow: { flex: 1, minWidth: 0, gap: 2 },
  track: { height: 6, borderRadius: 3, backgroundColor: m3.surfaceContainerHighest, overflow: 'hidden' },
  fill: { height: 6, borderRadius: 3, backgroundColor: m3.primary },
  actions: { flexDirection: 'row', flexWrap: 'wrap', gap: space.sm },
});
