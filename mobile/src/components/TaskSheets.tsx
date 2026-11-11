/**
 * Sheets for background tasks: add a prompt to the queue, and create or
 * edit a scheduled task.
 */
import { useEffect, useState } from 'react';
import { Pressable, ScrollView, StyleSheet, Text, View } from 'react-native';

import { describeError } from '../lib/api';
import { useClient } from '../lib/connection';
import { DAY_NAMES, EVERY_CHOICES, cleanDate, cleanTime, describeRepeat, hhmm, ymd } from '../lib/schedule';
import { CHATS_ID, type Project, type Repeat, type Schedule } from '../lib/types';
import { alert } from './Dialogs';
import { Sheet } from './Sheet';
import { m3, radius, space, type } from './theme';
import { Banner, Button, Field, Icon, type IconName } from './ui';

function Chip({
  label,
  on,
  onPress,
  icon,
}: {
  label: string;
  on: boolean;
  onPress: () => void;
  icon?: IconName;
}) {
  return (
    <Pressable
      onPress={onPress}
      style={({ pressed }) => [styles.chip, on && styles.chipOn, pressed && styles.pressed]}
      accessibilityRole="radio"
      accessibilityState={{ selected: on }}
      accessibilityLabel={label}
    >
      {on ? (
        <Icon name="check" size={16} color={m3.onSecondaryContainer} />
      ) : icon ? (
        <Icon name={icon} size={16} color={m3.onSurfaceVariant} />
      ) : null}
      <Text style={[type.labelLarge, { color: on ? m3.onSecondaryContainer : m3.onSurface }]} numberOfLines={1}>
        {label}
      </Text>
    </Pressable>
  );
}

/** Where the task runs: a plain chat or one of the projects. */
function WherePicker({
  projects,
  value,
  onChange,
}: {
  projects: Project[];
  value: string;
  onChange: (id: string) => void;
}) {
  return (
    <View style={styles.block}>
      <Text style={[type.bodySmall, styles.label]}>Runs in</Text>
      <View style={styles.chips}>
        <Chip label="Plain chat" icon="chat-outline" on={value === CHATS_ID} onPress={() => onChange(CHATS_ID)} />
        {projects.map((p) => (
          <Chip key={p.id} label={p.name} icon="folder-outline" on={value === p.id} onPress={() => onChange(p.id)} />
        ))}
      </View>
      <Text style={type.bodySmall}>
        {value === CHATS_ID
          ? 'A plain chat can search the web but has no access to your files.'
          : 'The agent can read, edit and run code in this project.'}
      </Text>
    </View>
  );
}

function useProjects(visible: boolean): Project[] {
  const client = useClient();
  const [projects, setProjects] = useState<Project[]>([]);
  useEffect(() => {
    if (!visible || !client) return;
    client
      .projects()
      .then(setProjects)
      .catch(() => setProjects([]));
  }, [visible, client]);
  return projects;
}

/** Adds a prompt to the queue: it runs in a new chat when the tasks before it are done. */
export function QueueSheet({
  visible,
  onClose,
  onAdded,
  initialProject,
}: {
  visible: boolean;
  onClose: () => void;
  onAdded: () => void;
  initialProject?: string;
}) {
  const client = useClient();
  const projects = useProjects(visible);
  const [where, setWhere] = useState(initialProject ?? CHATS_ID);
  const [name, setName] = useState('');
  const [prompt, setPrompt] = useState('');
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (visible) {
      setWhere(initialProject ?? CHATS_ID);
      setError(null);
    }
  }, [visible, initialProject]);

  const add = async () => {
    if (!client || !prompt.trim()) return;
    setSaving(true);
    setError(null);
    try {
      await client.enqueue({ prompt: prompt.trim(), name: name.trim() || undefined, project_id: where });
      setPrompt('');
      setName('');
      onAdded();
    } catch (e) {
      setError(describeError(e));
    } finally {
      setSaving(false);
    }
  };

  return (
    <Sheet visible={visible} onClose={onClose} title="Add to queue">
      <ScrollView style={styles.scroll} keyboardShouldPersistTaps="handled">
        <Text style={[type.bodyMedium, styles.intro]}>
          Queued tasks run one at a time on your PC, each in its own chat, even with this app closed. You get a
          notification when one finishes or needs your approval.
        </Text>
        <Field
          label="What should it do?"
          value={prompt}
          onChangeText={setPrompt}
          placeholder="Update the dependencies and fix anything that breaks"
          multiline
          autoCapitalize="sentences"
          autoCorrect
          autoFocus
          style={styles.prompt}
        />
        <Field
          label="Name (optional)"
          value={name}
          onChangeText={setName}
          placeholder="Taken from the first line if empty"
          autoCapitalize="sentences"
        />
        <WherePicker projects={projects} value={where} onChange={setWhere} />
        {error ? <Banner message={error} /> : null}
      </ScrollView>
      <View style={styles.actions}>
        <Button title="Cancel" variant="text" onPress={onClose} />
        <Button title="Add to queue" icon="tray-plus" onPress={add} loading={saving} disabled={!prompt.trim()} />
      </View>
    </Sheet>
  );
}

type Kind = Repeat['kind'];

/** Tomorrow at 08:00, as a starting point for a new schedule. */
function defaults() {
  const t = new Date();
  t.setDate(t.getDate() + 1);
  t.setHours(8, 0, 0, 0);
  return { date: ymd(t), time: hhmm(t) };
}

/** Creates or edits a scheduled task. */
export function ScheduleSheet({
  visible,
  onClose,
  onSaved,
  editing,
}: {
  visible: boolean;
  onClose: () => void;
  onSaved: () => void;
  /** The schedule to edit; null for a new one. */
  editing: Schedule | null;
}) {
  const client = useClient();
  const projects = useProjects(visible);
  const [name, setName] = useState('');
  const [prompt, setPrompt] = useState('');
  const [where, setWhere] = useState(CHATS_ID);
  const [kind, setKind] = useState<Kind>('daily');
  const [time, setTime] = useState('08:00');
  const [date, setDate] = useState(defaults().date);
  const [days, setDays] = useState<number[]>([]);
  const [minutes, setMinutes] = useState(60);
  const [sameChat, setSameChat] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!visible) return;
    setError(null);
    const d = defaults();
    const r = editing?.repeat;
    setName(editing?.name ?? '');
    setPrompt(editing?.prompt ?? '');
    setWhere(editing?.project_id ?? CHATS_ID);
    setSameChat(editing?.same_chat ?? false);
    setKind(r?.kind ?? 'daily');
    setTime(r?.kind === 'daily' ? r.time : r?.kind === 'once' ? (r.at.split('T')[1] ?? d.time).slice(0, 5) : d.time);
    setDate(r?.kind === 'once' ? r.at.split('T')[0] : d.date);
    setDays(r?.kind === 'daily' ? r.days : []);
    setMinutes(r?.kind === 'every' ? r.minutes : 60);
  }, [visible, editing]);

  const repeat = (): Repeat | string => {
    if (kind === 'every') return { kind, minutes };
    const t = cleanTime(time);
    if (!t) return 'Enter a time like 08:00';
    if (kind === 'daily') return { kind, time: t, days };
    const dt = cleanDate(date);
    if (!dt) return 'Enter a date like 2026-11-12';
    return { kind, at: `${dt}T${t}` };
  };
  const r = repeat();

  const save = async () => {
    if (!client) return;
    if (typeof r === 'string') {
      setError(r);
      return;
    }
    setSaving(true);
    setError(null);
    try {
      const body = { name: name.trim(), prompt: prompt.trim(), project_id: where, repeat: r, same_chat: sameChat };
      if (editing) await client.updateSchedule(editing.id, { ...body, enabled: true });
      else await client.createSchedule(body);
      onSaved();
    } catch (e) {
      setError(describeError(e));
    } finally {
      setSaving(false);
    }
  };

  const remove = () => {
    if (!client || !editing) return;
    alert(`Delete “${editing.name}”?`, 'It stops running. Chats from earlier runs are kept.', [
      { text: 'Cancel', style: 'cancel' },
      {
        text: 'Delete',
        style: 'destructive',
        onPress: async () => {
          try {
            await client.deleteSchedule(editing.id);
            onSaved();
          } catch (e) {
            setError(describeError(e));
          }
        },
      },
    ]);
  };

  // No days picked means every day, so the first tap turns one day off.
  const toggleDay = (d: number) =>
    setDays((cur) => {
      const all = cur.length ? cur : [1, 2, 3, 4, 5, 6, 7];
      const next = all.includes(d) ? all.filter((x) => x !== d) : [...all, d].sort();
      return next.length === 7 ? [] : next.length === 0 ? cur : next;
    });

  return (
    <Sheet visible={visible} onClose={onClose} title={editing ? 'Edit scheduled task' : 'New scheduled task'}>
      <ScrollView style={styles.scroll} keyboardShouldPersistTaps="handled">
        <Field
          label="Name"
          value={name}
          onChangeText={setName}
          placeholder="Morning summary"
          autoCapitalize="sentences"
        />
        <Field
          label="What should it do?"
          value={prompt}
          onChangeText={setPrompt}
          placeholder="Summarize what changed in this project since yesterday and list anything that needs me"
          multiline
          autoCapitalize="sentences"
          autoCorrect
          style={styles.prompt}
        />

        <View style={styles.block}>
          <Text style={[type.bodySmall, styles.label]}>When</Text>
          <View style={styles.chips}>
            <Chip label="Daily" icon="calendar-today" on={kind === 'daily'} onPress={() => setKind('daily')} />
            <Chip label="Repeat every" icon="repeat" on={kind === 'every'} onPress={() => setKind('every')} />
            <Chip label="Once" icon="calendar-clock" on={kind === 'once'} onPress={() => setKind('once')} />
          </View>
          {kind === 'daily' ? (
            <>
              <View style={styles.chips}>
                {DAY_NAMES.map((d, i) => (
                  <Pressable
                    key={d}
                    onPress={() => toggleDay(i + 1)}
                    style={[styles.day, (days.length === 0 || days.includes(i + 1)) && styles.dayOn]}
                    accessibilityRole="checkbox"
                    accessibilityState={{ checked: days.length === 0 || days.includes(i + 1) }}
                    accessibilityLabel={d}
                  >
                    <Text
                      style={[
                        type.labelMedium,
                        { color: days.length === 0 || days.includes(i + 1) ? m3.onPrimary : m3.onSurface },
                      ]}
                    >
                      {d.slice(0, 2)}
                    </Text>
                  </Pressable>
                ))}
              </View>
              <Text style={type.bodySmall}>
                {days.length === 0 ? 'Every day. Tap days to pick only some.' : 'Tap a day to turn it on or off.'}
              </Text>
            </>
          ) : null}
          {kind === 'every' ? (
            <View style={styles.chips}>
              {EVERY_CHOICES.map((c) => (
                <Chip key={c.minutes} label={c.label} on={minutes === c.minutes} onPress={() => setMinutes(c.minutes)} />
              ))}
            </View>
          ) : null}
        </View>
        {kind !== 'every' ? (
          <View style={styles.timeRow}>
            {kind === 'once' ? (
              <View style={styles.grow}>
                <Field label="Date" value={date} onChangeText={setDate} placeholder="2026-11-12" />
              </View>
            ) : null}
            <View style={styles.grow}>
              <Field label="Time (24-hour)" value={time} onChangeText={setTime} placeholder="08:00" keyboardType="numbers-and-punctuation" />
            </View>
          </View>
        ) : null}
        <Text style={[type.bodySmall, styles.summary]}>
          {typeof r === 'string' ? r : `${describeRepeat(r)}, in your PC's time zone.`}
        </Text>

        <WherePicker projects={projects} value={where} onChange={setWhere} />

        <View style={styles.block}>
          <Text style={[type.bodySmall, styles.label]}>Chats</Text>
          <View style={styles.chips}>
            <Chip label="New chat each run" on={!sameChat} onPress={() => setSameChat(false)} />
            <Chip label="Continue one chat" on={sameChat} onPress={() => setSameChat(true)} />
          </View>
          <Text style={type.bodySmall}>
            {sameChat
              ? 'Each run is a new message in the same chat, so the agent remembers earlier runs.'
              : 'Each run starts fresh in a chat of its own.'}
          </Text>
        </View>

        {error ? <Banner message={error} /> : null}
      </ScrollView>
      <View style={styles.actions}>
        {editing ? <Button title="Delete" variant="text" icon="delete-outline" onPress={remove} style={styles.left} /> : null}
        <Button title="Cancel" variant="text" onPress={onClose} />
        <Button
          title={editing ? 'Save' : 'Schedule'}
          icon="calendar-check"
          onPress={save}
          loading={saving}
          disabled={!prompt.trim()}
        />
      </View>
    </Sheet>
  );
}

const styles = StyleSheet.create({
  scroll: { maxHeight: 560 },
  intro: { marginBottom: space.md },
  prompt: { minHeight: 96, textAlignVertical: 'top' },
  block: { gap: space.sm, marginTop: space.md },
  label: { color: m3.onSurfaceVariant },
  chips: { flexDirection: 'row', flexWrap: 'wrap', gap: space.sm },
  chip: {
    flexDirection: 'row',
    alignItems: 'center',
    gap: 6,
    height: 36,
    paddingHorizontal: space.md,
    borderRadius: radius.sm,
    borderWidth: 1,
    borderColor: m3.outlineVariant,
    maxWidth: 260,
  },
  chipOn: { backgroundColor: m3.secondaryContainer, borderColor: m3.secondaryContainer },
  pressed: { backgroundColor: m3.pressed },
  day: {
    width: 40,
    height: 40,
    borderRadius: 20,
    alignItems: 'center',
    justifyContent: 'center',
    backgroundColor: m3.surfaceContainerHighest,
  },
  dayOn: { backgroundColor: m3.primary },
  timeRow: { flexDirection: 'row', gap: space.md, marginTop: space.sm },
  grow: { flex: 1, minWidth: 0 },
  summary: { marginTop: space.xs, color: m3.primary },
  actions: { flexDirection: 'row', justifyContent: 'flex-end', gap: space.sm, marginTop: space.md },
  left: { marginRight: 'auto' },
});
