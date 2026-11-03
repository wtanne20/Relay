import { useCallback, useEffect, useRef, useState } from 'react';
import { ActivityIndicator, Pressable, StyleSheet, Text, View } from 'react-native';

import { describeError, type RelayClient } from '../lib/api';
import type { ModelStatus } from '../lib/types';
import { m3, radius, space, type } from './theme';
import { Icon, type IconName } from './ui';

const POLL_MS = 5000;

/** Model status card; polls while the screen is focused. */
export function ModelStatusBar({
  client,
  active,
  onPress,
}: {
  client: RelayClient | null;
  active: boolean;
  onPress?: () => void;
}) {
  const [status, setStatus] = useState<ModelStatus | null>(null);
  const [error, setError] = useState<string | null>(null);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);

  const poll = useCallback(async () => {
    if (!client) return;
    try {
      setStatus(await client.model());
      setError(null);
    } catch (e) {
      setError(describeError(e));
    }
  }, [client]);

  useEffect(() => {
    if (!active) return;
    let alive = true;
    const loop = async () => {
      await poll();
      if (alive) timer.current = setTimeout(loop, POLL_MS);
    };
    loop();
    return () => {
      alive = false;
      if (timer.current) clearTimeout(timer.current);
    };
  }, [active, poll]);

  let tone = { bg: m3.surfaceContainerHighest, fg: m3.onSurfaceVariant };
  let icon: IconName = 'chip';
  let title = 'Checking model…';
  let detail: string | null = null;
  let busy = false;
  if (error) {
    tone = { bg: m3.errorContainer, fg: m3.onErrorContainer };
    icon = 'lan-disconnect';
    title = 'Can’t reach your PC';
    detail = error;
  } else if (status) {
    const name = status.model_path.split(/[\\/]/).pop() || 'model';
    if (status.state === 'ready') {
      tone = { bg: m3.successContainer, fg: m3.success };
      icon = 'check-circle-outline';
      title = 'Selected model';
      detail = name;
    } else if (status.state === 'starting') {
      tone = { bg: m3.primaryContainer, fg: m3.onPrimaryContainer };
      busy = true;
      title = status.progress != null ? 'Setting up' : 'Loading model';
      detail = status.message ?? name;
    } else {
      tone = { bg: m3.errorContainer, fg: m3.onErrorContainer };
      icon = 'alert-circle-outline';
      title = 'Model problem';
      detail = status.message ?? 'llama-server is not running';
    }
  }
  const progress = status?.state === 'starting' && status.progress != null ? status.progress : null;

  return (
    <Pressable
      onPress={onPress}
      disabled={!onPress}
      accessibilityRole="button"
      accessibilityLabel={`${title}. ${detail ?? ''}`}
      style={({ pressed }) => [styles.card, pressed && { opacity: 0.85 }]}
    >
      <View style={styles.row}>
        <View style={[styles.iconWrap, { backgroundColor: tone.bg }]}>
          {busy ? <ActivityIndicator size="small" color={tone.fg} /> : <Icon name={icon} size={22} color={tone.fg} />}
        </View>
        <View style={styles.textCol}>
          <Text style={type.titleMedium}>{title}</Text>
          {detail ? (
            <Text style={type.bodyMedium} numberOfLines={2}>
              {detail}
            </Text>
          ) : null}
        </View>
        {onPress ? <Icon name="chevron-right" color={m3.outline} /> : null}
      </View>
      {progress !== null ? (
        <View style={styles.track}>
          <View style={[styles.fill, { width: `${Math.round(progress * 100)}%` }]} />
        </View>
      ) : null}
    </Pressable>
  );
}

const styles = StyleSheet.create({
  card: {
    backgroundColor: m3.surfaceContainer,
    borderRadius: radius.xl,
    paddingHorizontal: space.lg,
    paddingVertical: space.md,
  },
  row: { flexDirection: 'row', alignItems: 'center', gap: space.lg, minHeight: 48 },
  iconWrap: { width: 44, height: 44, borderRadius: 14, alignItems: 'center', justifyContent: 'center' },
  textCol: { flex: 1, minWidth: 0 },
  track: {
    height: 4,
    borderRadius: 2,
    backgroundColor: m3.surfaceContainerHighest,
    marginTop: space.md,
    overflow: 'hidden',
  },
  fill: { height: 4, backgroundColor: m3.primary, borderRadius: 2 },
});
