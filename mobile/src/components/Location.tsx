/**
 * Where things run. Relay is spread over several computers on the tailnet:
 * one brain (it runs the model, the agent and keeps the chats) and any number
 * of linked computers whose folders the agent can work in. These labels make
 * both visible everywhere: which computer is the brain, and which computer
 * each project's (and so each chat's) files are on.
 */
import { useEffect, useState } from 'react';
import { Pressable, StyleSheet, Text, View } from 'react-native';

import { useConnection } from '../lib/connection';
import type { ProjectLocation, ServerInfo } from '../lib/types';
import { m3, radius, space, type } from './theme';
import { Icon, type IconName } from './ui';

let cachedInfo: { key: string; info: ServerInfo } | null = null;

/** The brain's /info (cached per connection). */
export function useBrainInfo(): ServerInfo | null {
  const { client, creds } = useConnection();
  const key = creds ? `${creds.url}|${creds.token}` : '';
  const [info, setInfo] = useState<ServerInfo | null>(cachedInfo?.key === key ? cachedInfo.info : null);
  useEffect(() => {
    if (!client || !key) return;
    let alive = true;
    client
      .info()
      .then((i) => {
        cachedInfo = { key, info: i };
        if (alive) setInfo(i);
      })
      .catch(() => undefined);
    return () => {
      alive = false;
    };
  }, [client, key]);
  return info;
}

/** Who "this computer" is, so labels can say "this computer" instead of a name. */
export function useHere(): { isBrain: boolean; deviceId: string | null } {
  const { mode } = useConnection();
  return {
    isBrain: mode?.mode === 'brain',
    deviceId: mode?.mode === 'device' ? mode.device_id : null,
  };
}

/** Plain words for a project's location ("on laptop", "on this computer"). */
export function locationLabel(
  loc: ProjectLocation | undefined,
  here: { isBrain: boolean; deviceId: string | null },
): { icon: IconName; text: string; online: boolean; brain: boolean } | null {
  if (!loc || loc.kind === 'none') return null;
  if (loc.kind === 'brain') {
    return {
      icon: 'brain',
      text: here.isBrain ? 'This computer (brain)' : `${loc.name || 'Brain'} (brain)`,
      online: true,
      brain: true,
    };
  }
  const self = here.deviceId === loc.device_id;
  return {
    icon: 'laptop',
    text: self ? `This computer (${loc.name})` : loc.name,
    online: loc.online,
    brain: false,
  };
}

/**
 * A small chip saying which computer a project's files are on, with a dot
 * for whether the brain can reach it right now.
 */
export function LocationChip({
  location,
  onPress,
  compact,
}: {
  location: ProjectLocation | undefined;
  onPress?: () => void;
  compact?: boolean;
}) {
  const here = useHere();
  const l = locationLabel(location, here);
  if (!l) return null;
  const body = (
    <>
      <Icon name={l.icon} size={compact ? 14 : 16} color={l.brain ? m3.primary : m3.onSurfaceVariant} />
      <Text style={[type.labelMedium, styles.chipText]} numberOfLines={1}>
        {compact ? l.text : `Files on ${l.text}`}
      </Text>
      {!l.brain ? (
        <View
          style={[styles.dot, { backgroundColor: l.online ? m3.success : m3.outline }]}
          accessibilityLabel={l.online ? 'connected' : 'offline'}
        />
      ) : null}
      {!l.online && !compact ? <Text style={[type.labelMedium, styles.offline]}>offline</Text> : null}
    </>
  );
  const label = `Files on ${l.text}${l.online ? '' : ', offline'}`;
  if (!onPress) {
    return (
      <View style={[styles.chip, compact && styles.chipCompact]} accessibilityLabel={label}>
        {body}
      </View>
    );
  }
  return (
    <Pressable
      onPress={onPress}
      style={({ pressed }) => [styles.chip, compact && styles.chipCompact, pressed && styles.pressed]}
      accessibilityRole="button"
      accessibilityLabel={label}
    >
      {body}
    </Pressable>
  );
}

/**
 * The brain this window talks to, and what this computer is. Shown at the
 * top of the sidebar and in Settings so it is always clear which computer
 * does the thinking.
 */
export function BrainBanner({ onPress }: { onPress?: () => void }) {
  const info = useBrainInfo();
  const { mode, client } = useConnection();
  if (!client) return null;
  const name = info?.hostname || (mode?.mode === 'device' ? mode.brain_name : '') || 'Brain';
  let title: string;
  let detail: string;
  if (mode?.mode === 'brain') {
    title = `${name} · this computer`;
    detail = 'This computer is the brain: it runs the model and keeps the chats.';
  } else if (mode?.mode === 'device') {
    const link = mode.link;
    title = `${name} · over Tailscale`;
    detail =
      link.state === 'connected'
        ? `This computer (${mode.name}) shares ${mode.root} with the brain.`
        : link.state === 'revoked'
          ? 'The brain removed this computer. Pair it again in Settings.'
          : link.state === 'offline'
            ? `This computer can't reach the brain right now (${link.message}).`
            : 'Connecting this computer to the brain…';
  } else {
    title = `${name} · over Tailscale`;
    detail = 'The brain runs the model and keeps the chats.';
  }
  const linkDot =
    mode?.mode === 'device'
      ? mode.link.state === 'connected'
        ? m3.success
        : mode.link.state === 'connecting'
          ? m3.warning
          : m3.error
      : null;
  const content = (
    <>
      <View style={styles.brainIcon}>
        <Icon name="brain" size={18} color={m3.onPrimaryContainer} />
      </View>
      <View style={styles.grow}>
        <Text style={[type.labelMedium, styles.brainKicker]}>BRAIN</Text>
        <Text style={type.labelLarge} numberOfLines={1}>
          {title}
        </Text>
        <Text style={[type.bodySmall, styles.brainDetail]} numberOfLines={2}>
          {detail}
        </Text>
      </View>
      {linkDot ? <View style={[styles.dot, { backgroundColor: linkDot }]} /> : null}
    </>
  );
  return onPress ? (
    <Pressable
      onPress={onPress}
      style={({ pressed }) => [styles.brain, pressed && styles.pressed]}
      accessibilityRole="button"
      accessibilityLabel={`Brain: ${title}. ${detail}`}
    >
      {content}
    </Pressable>
  ) : (
    <View style={styles.brain} accessibilityLabel={`Brain: ${title}. ${detail}`}>
      {content}
    </View>
  );
}

const styles = StyleSheet.create({
  grow: { flex: 1, minWidth: 0 },
  chip: {
    flexDirection: 'row',
    alignItems: 'center',
    alignSelf: 'flex-start',
    gap: 6,
    paddingHorizontal: space.md,
    paddingVertical: 6,
    borderRadius: radius.pill,
    backgroundColor: m3.surfaceContainerHigh,
    maxWidth: '100%',
  },
  chipCompact: { paddingHorizontal: space.sm, paddingVertical: 2, gap: 4, backgroundColor: 'transparent' },
  chipText: { color: m3.onSurfaceVariant, flexShrink: 1 },
  offline: { color: m3.outline },
  pressed: { opacity: 0.7 },
  dot: { width: 8, height: 8, borderRadius: 4 },
  brain: {
    flexDirection: 'row',
    alignItems: 'center',
    gap: space.md,
    padding: space.md,
    borderRadius: radius.lg,
    backgroundColor: m3.surfaceContainerHigh,
  },
  brainIcon: {
    width: 32,
    height: 32,
    borderRadius: 16,
    backgroundColor: m3.primaryContainer,
    alignItems: 'center',
    justifyContent: 'center',
  },
  brainKicker: { color: m3.primary, letterSpacing: 1 },
  brainDetail: { color: m3.onSurfaceVariant },
});
