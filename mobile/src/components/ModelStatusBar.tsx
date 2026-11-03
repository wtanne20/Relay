import { useCallback, useEffect, useRef, useState } from 'react';
import { ActivityIndicator, Pressable, StyleSheet, Text, View } from 'react-native';

import { describeError, type RelayClient } from '../lib/api';
import type { ModelStatus } from '../lib/types';
import { colors, radius, space, touch } from './theme';
import { Dot } from './ui';

const POLL_MS = 5000;

/** Model status indicator; polls while mounted and the screen is focused. */
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

  let color: string = colors.textMuted;
  let title = 'Checking model…';
  let detail: string | null = null;
  if (error) {
    color = colors.danger;
    title = 'Server unreachable';
    detail = error;
  } else if (status) {
    const name = status.model_path.split(/[\\/]/).pop() || 'model';
    if (status.state === 'ready') {
      color = colors.success;
      title = 'Model ready';
      detail = name;
    } else if (status.state === 'starting') {
      color = colors.warning;
      title = 'Model loading…';
      detail = status.message ?? name;
    } else {
      color = colors.danger;
      title = 'Model error';
      detail = status.message ?? 'llama-server is not running';
    }
  }

  return (
    <Pressable
      onPress={onPress}
      accessibilityRole="button"
      accessibilityLabel={`${title}. ${detail ?? ''}`}
      style={({ pressed }) => [styles.bar, pressed && { opacity: 0.8 }]}
    >
      {status?.state === 'starting' && !error ? <ActivityIndicator size="small" color={color} /> : <Dot color={color} />}
      <View style={styles.textCol}>
        <Text style={styles.title}>{title}</Text>
        {detail ? (
          <Text style={styles.detail} numberOfLines={2}>
            {detail}
          </Text>
        ) : null}
      </View>
      {onPress ? <Text style={styles.chevron}>›</Text> : null}
    </Pressable>
  );
}

const styles = StyleSheet.create({
  bar: {
    minHeight: touch + 8,
    flexDirection: 'row',
    alignItems: 'center',
    gap: space.md,
    backgroundColor: colors.surface,
    borderRadius: radius.lg,
    borderWidth: StyleSheet.hairlineWidth,
    borderColor: colors.border,
    paddingHorizontal: space.lg,
    paddingVertical: space.sm,
    marginHorizontal: space.lg,
    marginTop: space.md,
    marginBottom: space.sm,
  },
  textCol: { flex: 1 },
  title: { color: colors.text, fontSize: 15, fontWeight: '600' },
  detail: { color: colors.textMuted, fontSize: 13, marginTop: 2 },
  chevron: { color: colors.textFaint, fontSize: 24 },
});
