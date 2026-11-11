import { router, useFocusEffect } from 'expo-router';
import { useCallback, useState } from 'react';
import { ActivityIndicator, Pressable, RefreshControl, ScrollView, StyleSheet, Switch, Text, View } from 'react-native';

import { alert } from '../components/Dialogs';
import { QueueSheet, ScheduleSheet } from '../components/TaskSheets';
import { m3, radius, space, statusColor, type } from '../components/theme';
import { Banner, Button, Icon, IconButton, type IconName } from '../components/ui';
import { describeError } from '../lib/api';
import { useClient } from '../lib/connection';
import { emit } from '../lib/events';
import { relativeTime } from '../lib/format';
import { usePageStyle, useWide } from '../lib/layout';
import { describeRepeat, queueStatusText, whenLabel } from '../lib/schedule';
import { CHATS_ID, type QueueItem, type Schedule } from '../lib/types';

const POLL_MS = 3000;

function itemIcon(item: QueueItem): { name: IconName; color: string } {
  switch (item.status) {
    case 'queued':
      return { name: 'clock-outline', color: m3.onSurfaceVariant };
    case 'running':
      return item.needs_approval
        ? { name: 'hand-back-right-outline', color: m3.warning }
        : { name: 'progress-clock', color: statusColor.running };
    case 'done':
      return { name: 'check-circle-outline', color: statusColor.done };
    case 'error':
      return { name: 'alert-circle-outline', color: statusColor.error };
    case 'stopped':
      return { name: 'stop-circle-outline', color: statusColor.stopped };
  }
}

/**
 * Background tasks: the queue (running, waiting and recently finished) and
 * scheduled tasks. Everything runs on the PC, with or without this app open.
 */
export default function TasksScreen() {
  const page = usePageStyle();
  const wide = useWide();
  const client = useClient();
  const [items, setItems] = useState<QueueItem[] | null>(null);
  const [schedules, setSchedules] = useState<Schedule[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [queueing, setQueueing] = useState(false);
  const [editing, setEditing] = useState<Schedule | null | 'new'>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [refreshing, setRefreshing] = useState(false);

  const load = useCallback(async () => {
    if (!client) return;
    try {
      const [q, s] = await Promise.all([client.queue(), client.schedules()]);
      setItems(q.items);
      setSchedules(s);
      setError(null);
    } catch (e) {
      setError(
        e && typeof e === 'object' && 'status' in e && (e as { status?: number }).status === 404
          ? 'Your PC runs an older Relay without background tasks. Update Relay on the PC to use them.'
          : describeError(e),
      );
    }
  }, [client]);

  useFocusEffect(
    useCallback(() => {
      load();
      const t = setInterval(load, POLL_MS);
      return () => clearInterval(t);
    }, [load]),
  );

  if (!client) return null;
  const open = (href: Parameters<typeof router.push>[0]) => (wide ? router.replace(href) : router.push(href));

  const run = async (key: string, act: () => Promise<unknown>) => {
    setBusy(key);
    try {
      await act();
      emit('chats');
      await load();
    } catch (e) {
      alert('Something went wrong', describeError(e));
    } finally {
      setBusy(null);
    }
  };

  const remove = (item: QueueItem) => {
    if (item.status === 'running') {
      alert(`Stop “${item.name}”?`, 'The agent stops where it is. The chat is kept.', [
        { text: 'Cancel', style: 'cancel' },
        { text: 'Stop', style: 'destructive', onPress: () => run(item.id, () => client.removeQueued(item.id)) },
      ]);
    } else {
      run(item.id, () => client.removeQueued(item.id));
    }
  };

  const active = (items ?? []).filter((i) => i.status === 'running' || i.status === 'queued');
  const finished = (items ?? []).filter((i) => i.status !== 'running' && i.status !== 'queued');
  let place = 0;

  const queueRow = (item: QueueItem, i: number) => {
    const icon = itemIcon(item);
    if (item.status === 'queued') place += 1;
    const status = queueStatusText(item, place);
    const time =
      item.status === 'queued'
        ? `added ${relativeTime(item.created_at)}`
        : item.status === 'running'
          ? item.started_at
            ? `started ${relativeTime(item.started_at)}`
            : ''
          : item.finished_at
            ? relativeTime(item.finished_at)
            : '';
    const canOpen = !!item.session_id;
    return (
      <View key={item.id}>
        {i > 0 ? <View style={styles.divider} /> : null}
        <View style={styles.row}>
          <Pressable
            onPress={() =>
              canOpen &&
              open({ pathname: '/session/[id]', params: { id: item.session_id!, title: item.session_title ?? item.name } })
            }
            disabled={!canOpen}
            style={({ hovered, pressed }: { pressed: boolean; hovered?: boolean }) => [
              styles.item,
              canOpen && (hovered || pressed) && styles.hover,
            ]}
            accessibilityRole={canOpen ? 'button' : undefined}
            accessibilityLabel={`${item.name}. ${status}`}
          >
            {item.status === 'running' && !item.needs_approval ? (
              <ActivityIndicator size="small" color={icon.color} style={styles.icon} />
            ) : (
              <Icon name={icon.name} size={22} color={icon.color} />
            )}
            <View style={styles.grow}>
              <Text style={type.bodyLarge} numberOfLines={1}>
                {item.name || 'Task'}
              </Text>
              <Text
                style={[type.bodySmall, item.needs_approval && { color: m3.warning }]}
                numberOfLines={2}
              >
                {status}
                {item.project_id === CHATS_ID ? '' : ` · ${item.project_name}`}
                {item.schedule_id ? ' · scheduled' : ''}
                {time ? ` · ${time}` : ''}
              </Text>
            </View>
          </Pressable>
          {item.status === 'queued' && place > 1 ? (
            <IconButton
              icon="arrow-collapse-up"
              label={`Run ${item.name} next`}
              size={20}
              disabled={busy === item.id}
              onPress={() => run(item.id, () => client.runQueuedFirst(item.id))}
            />
          ) : null}
          <IconButton
            icon={item.status === 'running' ? 'stop' : 'close'}
            label={item.status === 'running' ? `Stop ${item.name}` : item.status === 'queued' ? `Cancel ${item.name}` : `Hide ${item.name}`}
            size={20}
            disabled={busy === item.id}
            onPress={() => remove(item)}
          />
        </View>
      </View>
    );
  };

  const scheduleRow = (s: Schedule, i: number) => (
    <View key={s.id}>
      {i > 0 ? <View style={styles.divider} /> : null}
      <View style={styles.row}>
        <Pressable
          onPress={() => setEditing(s)}
          style={({ hovered, pressed }: { pressed: boolean; hovered?: boolean }) => [
            styles.item,
            (hovered || pressed) && styles.hover,
          ]}
          accessibilityRole="button"
          accessibilityLabel={`${s.name}. ${describeRepeat(s.repeat)}. Edit`}
        >
          <Icon name={s.enabled ? 'calendar-clock' : 'calendar-remove-outline'} size={22} color={s.enabled ? m3.primary : m3.outline} />
          <View style={styles.grow}>
            <Text style={[type.bodyLarge, !s.enabled && { color: m3.onSurfaceVariant }]} numberOfLines={1}>
              {s.name}
            </Text>
            <Text style={type.bodySmall} numberOfLines={2}>
              {describeRepeat(s.repeat)}
              {s.project_id === CHATS_ID ? '' : ` · ${s.project_name}`}
            </Text>
            <Text style={type.bodySmall} numberOfLines={1}>
              {s.enabled && s.next_run ? `Next ${whenLabel(s.next_run)}` : s.enabled ? '' : 'Off'}
              {s.last_run ? ` · last ran ${relativeTime(s.last_run)}` : ''}
              {s.last_status === 'error' ? ' (failed)' : ''}
            </Text>
          </View>
        </Pressable>
        {s.last_session_id ? (
          <IconButton
            icon="message-text-outline"
            label={`Open the latest run of ${s.name}`}
            size={20}
            onPress={() => open({ pathname: '/session/[id]', params: { id: s.last_session_id!, title: s.name } })}
          />
        ) : null}
        <IconButton
          icon="play-outline"
          label={`Run ${s.name} now`}
          size={20}
          disabled={busy === s.id}
          onPress={() => run(s.id, () => client.runSchedule(s.id))}
        />
        <Switch
          value={s.enabled}
          onValueChange={(on) => run(s.id, () => client.updateSchedule(s.id, { enabled: on }))}
          disabled={busy === s.id}
          trackColor={{ true: m3.primary, false: m3.surfaceContainerHighest }}
          thumbColor={s.enabled ? m3.onPrimary : m3.outline}
          {...({ activeThumbColor: m3.onPrimary } as object)}
          accessibilityLabel={`${s.name} on`}
          style={styles.switch}
        />
      </View>
    </View>
  );

  return (
    <>
      <ScrollView
        contentContainerStyle={[styles.content, page]}
        refreshControl={
          wide ? undefined : (
            <RefreshControl
              refreshing={refreshing}
              onRefresh={async () => {
                setRefreshing(true);
                await load();
                setRefreshing(false);
              }}
              tintColor={m3.onSurfaceVariant}
            />
          )
        }
      >
        {error ? <Banner message={error} actionLabel="Retry" onAction={load} /> : null}

        <View style={styles.sectionRow}>
          <Text style={[type.titleSmall, styles.section]}>Queue</Text>
          <Button title="Add to queue" icon="tray-plus" variant="tonal" onPress={() => setQueueing(true)} />
        </View>
        {items === null && !error ? <ActivityIndicator color={m3.primary} /> : null}
        {items && active.length === 0 ? (
          <Text style={[type.bodyMedium, styles.empty]}>
            Nothing waiting. Queue a few jobs and they run one after another on your PC, even with this app closed.
            You get a notification when each one finishes or needs you.
          </Text>
        ) : null}
        {active.length ? <View style={styles.group}>{active.map(queueRow)}</View> : null}

        {finished.length ? (
          <>
            <View style={styles.sectionRow}>
              <Text style={[type.titleSmall, styles.section]}>Finished</Text>
              <Button
                title="Clear"
                variant="text"
                onPress={() => run('clear', () => client.clearFinishedTasks())}
                loading={busy === 'clear'}
              />
            </View>
            <View style={styles.group}>{finished.map(queueRow)}</View>
          </>
        ) : null}

        <View style={styles.sectionRow}>
          <Text style={[type.titleSmall, styles.section]}>Scheduled</Text>
          <Button title="New" icon="calendar-plus" variant="tonal" onPress={() => setEditing('new')} />
        </View>
        {schedules?.length === 0 ? (
          <Text style={[type.bodyMedium, styles.empty]}>
            Run a prompt on a schedule, like a summary of a project every weekday morning. Each run joins the queue
            above.
          </Text>
        ) : null}
        {schedules?.length ? <View style={styles.group}>{schedules.map(scheduleRow)}</View> : null}
      </ScrollView>

      <QueueSheet
        visible={queueing}
        onClose={() => setQueueing(false)}
        onAdded={() => {
          setQueueing(false);
          load();
        }}
      />
      <ScheduleSheet
        visible={editing !== null}
        editing={editing === 'new' ? null : editing}
        onClose={() => setEditing(null)}
        onSaved={() => {
          setEditing(null);
          load();
        }}
      />
    </>
  );
}

const styles = StyleSheet.create({
  content: { padding: space.lg, gap: space.md, paddingBottom: 64 },
  sectionRow: {
    flexDirection: 'row',
    alignItems: 'center',
    justifyContent: 'space-between',
    marginTop: space.sm,
    minHeight: 40,
  },
  section: { color: m3.onSurfaceVariant, marginLeft: space.xs },
  empty: { paddingHorizontal: space.xs },
  group: { backgroundColor: m3.surfaceContainerLow, borderRadius: radius.lg, overflow: 'hidden' },
  divider: { height: 1, backgroundColor: m3.outlineVariant, marginLeft: 56 },
  row: { flexDirection: 'row', alignItems: 'center', paddingRight: space.sm },
  item: {
    flex: 1,
    minWidth: 0,
    flexDirection: 'row',
    alignItems: 'center',
    gap: space.md,
    minHeight: 64,
    paddingHorizontal: space.lg,
    paddingVertical: space.sm,
  },
  icon: { width: 22 },
  grow: { flex: 1, minWidth: 0 },
  hover: { backgroundColor: m3.hover },
  switch: { marginLeft: space.xs },
});
