import { router, useFocusEffect, useLocalSearchParams } from 'expo-router';
import { useCallback, useRef, useState } from 'react';
import { ActivityIndicator, Pressable, ScrollView, StyleSheet, Text, View } from 'react-native';

import { alert } from '../components/Dialogs';
import { m3, mono, radius, space, type } from '../components/theme';
import { Banner, Button, Icon } from '../components/ui';
import { describeError } from '../lib/api';
import { useClient } from '../lib/connection';
import { relativeTime } from '../lib/format';
import { usePageStyle } from '../lib/layout';
import { DECISION_LABEL, TOOL_ICON, TOOL_VERB } from '../lib/rules';
import { CHATS_ID, type ActivityFilter, type AuditDecision, type AuditEntry } from '../lib/types';

const FILTERS: { value: ActivityFilter; label: string }[] = [
  { value: '', label: 'All' },
  { value: 'commands', label: 'Commands' },
  { value: 'files', label: 'File changes' },
  { value: 'web', label: 'Web' },
  { value: 'asked', label: 'Asked you' },
  { value: 'denied', label: 'Denied' },
];

const DECISION_COLOR: Partial<Record<AuditDecision, { bg: string; fg: string }>> = {
  rule: { bg: m3.successContainer, fg: m3.success },
  allowed: { bg: m3.secondaryContainer, fg: m3.onSecondaryContainer },
  allowed_all: { bg: m3.warningContainer, fg: m3.onWarningContainer },
  denied: { bg: m3.errorContainer, fg: m3.onErrorContainer },
};

/**
 * Everything the agent did with its tools, newest first: what it ran,
 * changed, read or opened, in which chat, and how it was allowed. Entries
 * stay after a chat is deleted.
 */
export default function ActivityScreen() {
  const page = usePageStyle();
  const client = useClient();
  const params = useLocalSearchParams<{ session?: string; title?: string }>();
  const [filter, setFilter] = useState<ActivityFilter>('');
  const [entries, setEntries] = useState<AuditEntry[] | null>(null);
  const [more, setMore] = useState(false);
  const [loadingMore, setLoadingMore] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const request = useRef(0);

  const load = useCallback(
    async (f: ActivityFilter) => {
      if (!client) return;
      const n = ++request.current;
      try {
        const r = await client.activity({ filter: f, sessionId: params.session });
        if (n !== request.current) return;
        setEntries(r.entries);
        setMore(r.has_more);
        setError(null);
      } catch (e) {
        if (n === request.current) setError(describeError(e));
      }
    },
    [client, params.session],
  );

  useFocusEffect(
    useCallback(() => {
      load(filter);
    }, [load, filter]),
  );

  if (!client) return null;

  const loadMore = async () => {
    const last = entries?.[entries.length - 1];
    if (!last) return;
    setLoadingMore(true);
    try {
      const r = await client.activity({ filter, sessionId: params.session, before: last.id });
      setEntries((es) => [...(es ?? []), ...r.entries]);
      setMore(r.has_more);
    } catch (e) {
      setError(describeError(e));
    } finally {
      setLoadingMore(false);
    }
  };

  const clear = () =>
    alert('Clear all activity?', 'The history of what the agent did is deleted for good. Chats are kept.', [
      { text: 'Cancel', style: 'cancel' },
      {
        text: 'Clear',
        style: 'destructive',
        onPress: async () => {
          try {
            await client.clearActivity();
            setEntries([]);
            setMore(false);
          } catch (e) {
            alert('Could not clear activity', describeError(e));
          }
        },
      },
    ]);

  return (
    <ScrollView contentContainerStyle={[styles.content, page]}>
      {params.session ? (
        <Text style={[type.bodyMedium, styles.muted]} numberOfLines={1}>
          In “{params.title ?? 'this chat'}”
        </Text>
      ) : null}
      <ScrollView horizontal showsHorizontalScrollIndicator={false} contentContainerStyle={styles.filters}>
        {FILTERS.map((f) => {
          const on = f.value === filter;
          return (
            <Pressable
              key={f.value || 'all'}
              onPress={() => {
                setFilter(f.value);
                setEntries(null);
              }}
              style={[styles.chip, on && styles.chipOn]}
              accessibilityRole="radio"
              accessibilityState={{ selected: on }}
              accessibilityLabel={f.label}
            >
              {on ? <Icon name="check" size={16} color={m3.onSecondaryContainer} /> : null}
              <Text style={[type.labelLarge, { color: on ? m3.onSecondaryContainer : m3.onSurface }]}>{f.label}</Text>
            </Pressable>
          );
        })}
      </ScrollView>

      {error ? <Banner message={error} actionLabel="Retry" onAction={() => load(filter)} /> : null}
      {!entries && !error ? <ActivityIndicator color={m3.primary} style={styles.pad} /> : null}
      {entries && !entries.length ? (
        <View style={styles.empty}>
          <Icon name="history" size={40} color={m3.outline} />
          <Text style={[type.bodyLarge, styles.muted]}>Nothing here yet</Text>
          <Text style={[type.bodyMedium, styles.muted]}>
            Commands the agent runs, files it changes and pages it opens show up here.
          </Text>
        </View>
      ) : null}

      {entries?.length ? (
        <View style={styles.group}>
          {entries.map((e, i) => (
            <View key={e.id}>
              {i > 0 ? <View style={styles.divider} /> : null}
              <Entry entry={e} showChat={!params.session} />
            </View>
          ))}
        </View>
      ) : null}

      {more ? (
        <Button title="Show older" variant="text" onPress={loadMore} loading={loadingMore} style={styles.more} />
      ) : null}
      {entries?.length && !params.session ? (
        <Button title="Clear activity" icon="delete-outline" variant="text" onPress={clear} style={styles.more} />
      ) : null}
    </ScrollView>
  );
}

function Entry({ entry: e, showChat }: { entry: AuditEntry; showChat: boolean }) {
  const color = DECISION_COLOR[e.decision];
  const where = [e.project_id === CHATS_ID ? null : e.project_name, showChat ? e.session_title : null]
    .filter(Boolean)
    .join(' · ');
  const open = e.session_exists
    ? () => router.push({ pathname: '/session/[id]', params: { id: e.session_id, title: e.session_title } })
    : undefined;
  return (
    <Pressable
      onPress={open}
      disabled={!open}
      style={({ pressed, hovered }: { pressed: boolean; hovered?: boolean }) => [
        styles.row,
        open && (pressed || hovered) && styles.hover,
      ]}
      accessibilityRole={open ? 'button' : undefined}
      accessibilityLabel={`${TOOL_VERB[e.tool] ?? e.tool} ${e.detail}. ${DECISION_LABEL[e.decision]}`}
    >
      <Icon name={TOOL_ICON[e.tool] ?? 'tools'} size={20} color={e.is_error ? m3.error : m3.onSurfaceVariant} />
      <View style={styles.grow}>
        <Text style={styles.detail} numberOfLines={3}>
          {e.detail || e.tool}
        </Text>
        <View style={styles.meta}>
          <View style={[styles.badge, { backgroundColor: color?.bg ?? m3.surfaceContainerHighest }]}>
            <Text style={[type.labelMedium, { color: color?.fg ?? m3.onSurfaceVariant }]}>{DECISION_LABEL[e.decision]}</Text>
          </View>
          {e.is_error && e.decision !== 'denied' && e.decision !== 'stopped' && e.decision !== 'off' ? (
            <Text style={[type.labelMedium, { color: m3.error }]}>failed</Text>
          ) : null}
          <Text style={[type.bodySmall, styles.grow]} numberOfLines={1}>
            {relativeTime(e.at)}
            {where ? ` · ${where}` : ''}
          </Text>
        </View>
      </View>
    </Pressable>
  );
}

const styles = StyleSheet.create({
  content: { padding: space.lg, paddingBottom: 64, gap: space.md },
  muted: { color: m3.onSurfaceVariant, textAlign: 'center' },
  pad: { margin: space.xl },
  filters: { gap: space.sm, paddingVertical: space.xs },
  chip: {
    flexDirection: 'row',
    alignItems: 'center',
    gap: space.xs,
    height: 32,
    paddingHorizontal: space.md,
    borderRadius: radius.sm,
    borderWidth: 1,
    borderColor: m3.outlineVariant,
  },
  chipOn: { backgroundColor: m3.secondaryContainer, borderColor: m3.secondaryContainer },
  empty: { alignItems: 'center', gap: space.sm, padding: space.xl },
  group: { backgroundColor: m3.surfaceContainer, borderRadius: radius.lg, overflow: 'hidden' },
  divider: { height: StyleSheet.hairlineWidth, backgroundColor: m3.outlineVariant, marginLeft: 52 },
  row: { flexDirection: 'row', gap: space.lg, paddingHorizontal: space.lg, paddingVertical: space.md },
  hover: { backgroundColor: m3.hover },
  grow: { flex: 1, minWidth: 0 },
  detail: { fontFamily: mono, fontSize: 13, lineHeight: 19, color: m3.onSurface },
  meta: { flexDirection: 'row', alignItems: 'center', gap: space.sm, marginTop: space.xs },
  badge: { paddingHorizontal: space.sm, paddingVertical: 2, borderRadius: radius.xs },
  more: { alignSelf: 'center' },
});
