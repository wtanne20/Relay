import { router, useFocusEffect } from 'expo-router';
import { useCallback, useMemo, useRef, useState } from 'react';
import {
  ActivityIndicator,
  Alert,
  KeyboardAvoidingView,
  Platform,
  ScrollView,
  StyleSheet,
  Text,
  View,
} from 'react-native';
import { useSafeAreaInsets } from 'react-native-safe-area-context';

import { Group, NavRow, NumberRow, SectionTitle, Segmented, TextRow, ToggleRow } from '../components/SettingsControls';
import { colors, space } from '../components/theme';
import { Banner, Button } from '../components/ui';
import { describeError } from '../lib/api';
import { useConnection } from '../lib/connection';
import { basename } from '../lib/format';
import type { ApprovalMode, Settings, SettingsResponse, SkillInfo } from '../lib/types';

const APPROVAL_OPTIONS: { value: ApprovalMode; label: string }[] = [
  { value: 'never', label: 'Never' },
  { value: 'commands', label: 'Commands' },
  { value: 'all_changes', label: 'All changes' },
];

const APPROVAL_HINT: Record<ApprovalMode, string> = {
  never: 'The agent works on its own until the task is done.',
  commands: 'Ask me before running any command.',
  all_changes: 'Ask me before running commands or changing files.',
};

const COMMON_CACHE = ['f16', 'q8_0', 'q4_0'];

function same(a: unknown, b: unknown) {
  return JSON.stringify(a) === JSON.stringify(b);
}

/** Every agent and model setting, saved to the PC's config.toml. */
export default function AgentSettingsScreen() {
  const { client } = useConnection();
  const insets = useSafeAreaInsets();
  const [meta, setMeta] = useState<SettingsResponse | null>(null);
  const [saved, setSaved] = useState<Settings | null>(null);
  const [draft, setDraft] = useState<Settings | null>(null);
  const [skills, setSkills] = useState<SkillInfo[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [invalid, setInvalid] = useState<Record<string, boolean>>({});
  const [extraArgs, setExtraArgs] = useState('');
  const dirtyRef = useRef(false);

  const load = useCallback(async () => {
    if (!client) return;
    try {
      const [s, sk] = await Promise.all([client.settings(), client.skills().catch(() => [] as SkillInfo[])]);
      setMeta(s);
      setSaved(s.settings);
      setDraft(s.settings);
      setExtraArgs(s.settings.llama.extra_args.join(' '));
      setSkills(sk);
      setInvalid({});
      setError(null);
    } catch (e) {
      setError(describeError(e));
    }
  }, [client]);

  useFocusEffect(
    useCallback(() => {
      // Reload when coming back (e.g. after picking a model), unless there
      // are unsaved edits.
      if (!dirtyRef.current) load();
    }, [load]),
  );

  const dirty = useMemo(() => !!draft && !!saved && !same(draft, saved), [draft, saved]);
  dirtyRef.current = dirty;
  const llamaChanged = !!draft && !!saved && !same(draft.llama, saved.llama);
  const hasInvalid = Object.values(invalid).some(Boolean);

  if (!client) return null;
  if (!draft || !meta || !saved) {
    return (
      <View style={styles.center}>
        {error ? (
          <Banner message={error} actionLabel="Retry" onAction={load} />
        ) : (
          <ActivityIndicator color={colors.accent} />
        )}
      </View>
    );
  }

  const set = <K extends keyof Settings>(section: K, patch: Partial<Settings[K]>) =>
    setDraft((d) =>
      d ? { ...d, [section]: Array.isArray(d[section]) ? patch : { ...(d[section] as object), ...patch } } : d,
    );
  const validity = (key: string) => (ok: boolean) => setInvalid((v) => (v[key] === !ok ? v : { ...v, [key]: !ok }));

  const save = async () => {
    const patch: Partial<Settings> = {};
    for (const k of Object.keys(draft) as (keyof Settings)[]) {
      if (!same(draft[k], saved[k])) (patch as Record<string, unknown>)[k] = draft[k];
    }
    setSaving(true);
    try {
      const res = await client.patchSettings(patch);
      setSaved(res.settings);
      setDraft(res.settings);
      setExtraArgs(res.settings.llama.extra_args.join(' '));
      setError(null);
      if (res.model_restarting) {
        Alert.alert('Saved', 'The model is reloading with the new settings. This can take a minute.');
      }
    } catch (e) {
      setError(describeError(e));
    } finally {
      setSaving(false);
    }
  };

  const discard = () => {
    setDraft(saved);
    setExtraArgs(saved.llama.extra_args.join(' '));
    setInvalid({});
  };

  const cacheOptions = (current: string) =>
    (COMMON_CACHE.includes(current) ? COMMON_CACHE : [...COMMON_CACHE, current]).map((v) => ({ value: v, label: v }));
  const vram = meta.gpus.reduce((n, g) => n + g.vram_mb, 0);
  const gpuText = meta.gpus.length
    ? meta.gpus.map((g) => `${g.name} (${Math.round(g.vram_mb / 1024)} GB)`).join(', ')
    : 'No NVIDIA GPU detected';

  const a = draft.agent;
  const s = draft.sampling;
  const l = draft.llama;
  const disabled = new Set(draft.disabled_skills);

  return (
    <KeyboardAvoidingView style={styles.flex} behavior={Platform.OS === 'ios' ? 'padding' : undefined}>
      <ScrollView style={styles.flex} contentContainerStyle={styles.content} keyboardShouldPersistTaps="handled">
        {error ? <Banner message={error} /> : null}

        <SectionTitle title="Model" />
        <Group>
          <NavRow
            label="Model"
            value={l.model_path ? basename(l.model_path) : 'Pick automatically'}
            onPress={() => router.push('/models')}
          />
          <View style={styles.note}>
            <Text style={styles.noteText}>{gpuText}</Text>
          </View>
        </Group>

        <SectionTitle title="Behaviour" />
        <Group>
          <Segmented
            label="Ask before acting"
            hint={APPROVAL_HINT[a.approval]}
            value={a.approval}
            options={APPROVAL_OPTIONS}
            onChange={(v) => set('agent', { approval: v })}
          />
          <ToggleRow
            label="Thinking"
            hint="Let reasoning models think before they answer. Slower, usually smarter."
            value={a.thinking}
            onChange={(v) => set('agent', { thinking: v })}
          />
          <NumberRow
            label="Max steps per prompt"
            hint="Model turns before the agent stops."
            value={a.max_steps}
            min={1}
            max={500}
            integer
            onChange={(v) => set('agent', { max_steps: v })}
            onValidity={validity('max_steps')}
          />
          <NumberRow
            label="Command timeout (s)"
            hint="Default limit for each command."
            value={a.command_timeout_seconds}
            min={1}
            max={3600}
            integer
            onChange={(v) => set('agent', { command_timeout_seconds: v })}
            onValidity={validity('timeout')}
          />
        </Group>

        <SectionTitle title="Tools" />
        <Group>
          {meta.tools.map((t) => (
            <ToggleRow
              key={t.name}
              label={t.description}
              hint={t.name}
              value={a.tools[t.name]}
              onChange={(v) => set('agent', { tools: { ...a.tools, [t.name]: v } })}
            />
          ))}
        </Group>

        <SectionTitle
          title="Sampling"
          action={
            same(s, meta.defaults.sampling)
              ? undefined
              : { label: 'Reset', onPress: () => set('sampling', meta.defaults.sampling) }
          }
        />
        <Group>
          <NumberRow
            label="Temperature"
            hint="Higher is more creative."
            value={s.temperature}
            min={0}
            max={2}
            onChange={(v) => set('sampling', { temperature: v })}
            onValidity={validity('temperature')}
          />
          <NumberRow
            label="Top P"
            value={s.top_p}
            min={0}
            max={1}
            onChange={(v) => set('sampling', { top_p: v })}
            onValidity={validity('top_p')}
          />
          <NumberRow
            label="Top K"
            value={s.top_k}
            min={0}
            max={1000}
            integer
            onChange={(v) => set('sampling', { top_k: v })}
            onValidity={validity('top_k')}
          />
          <NumberRow
            label="Min P"
            value={s.min_p}
            min={0}
            max={1}
            onChange={(v) => set('sampling', { min_p: v })}
            onValidity={validity('min_p')}
          />
          <NumberRow
            label="Repeat penalty"
            value={s.repeat_penalty}
            min={0.5}
            max={2}
            onChange={(v) => set('sampling', { repeat_penalty: v })}
            onValidity={validity('repeat')}
          />
          <NumberRow
            label="Presence penalty"
            value={s.presence_penalty}
            min={-2}
            max={2}
            onChange={(v) => set('sampling', { presence_penalty: v })}
            onValidity={validity('presence')}
          />
          <NumberRow
            label="Max reply tokens"
            hint="0 means no limit."
            value={s.max_tokens}
            min={0}
            max={1048576}
            integer
            onChange={(v) => set('sampling', { max_tokens: v })}
            onValidity={validity('max_tokens')}
          />
        </Group>

        <SectionTitle
          title="Model loading"
          action={
            same(
              { ...l, model_path: '', llama_server_path: '' },
              { ...meta.defaults.llama, model_path: '', llama_server_path: '' },
            )
              ? undefined
              : {
                  label: 'Reset',
                  onPress: () => {
                    set('llama', {
                      ...meta.defaults.llama,
                      model_path: l.model_path,
                      llama_server_path: l.llama_server_path,
                      models_dirs: l.models_dirs,
                    });
                    setExtraArgs('');
                  },
                }
          }
        />
        <Group>
          <NumberRow
            label="Context length"
            hint="Tokens the model can see at once. More uses more VRAM."
            value={l.context_size}
            min={512}
            max={1048576}
            integer
            onChange={(v) => set('llama', { context_size: v })}
            onValidity={validity('ctx')}
          />
          <NumberRow
            label="GPU layers"
            hint={vram ? '99 puts the whole model on the GPU.' : 'No GPU found: 0 runs on the CPU.'}
            value={l.gpu_layers}
            min={0}
            max={999}
            integer
            onChange={(v) => set('llama', { gpu_layers: v })}
            onValidity={validity('ngl')}
          />
          <Segmented
            label="Flash attention"
            value={l.flash_attn}
            options={[
              { value: 'on', label: 'On' },
              { value: 'auto', label: 'Auto' },
              { value: 'off', label: 'Off' },
            ]}
            onChange={(v) => set('llama', { flash_attn: v })}
          />
          <Segmented
            label="KV cache (K)"
            hint="q8_0 halves cache memory with little quality loss."
            value={l.cache_type_k}
            options={cacheOptions(l.cache_type_k)}
            onChange={(v) => set('llama', { cache_type_k: v })}
          />
          <Segmented
            label="KV cache (V)"
            value={l.cache_type_v}
            options={cacheOptions(l.cache_type_v)}
            onChange={(v) => set('llama', { cache_type_v: v })}
          />
          <TextRow
            label="Extra llama-server arguments"
            hint="Advanced, e.g. --threads 8 --mlock"
            value={extraArgs}
            placeholder="none"
            onChange={(t) => {
              setExtraArgs(t);
              set('llama', { extra_args: t.split(/\s+/).filter(Boolean) });
            }}
          />
        </Group>

        <SectionTitle title="Prompt and skills" />
        <Group>
          <NavRow label="System prompt" onPress={() => router.push('/system-prompt')} />
          {skills && skills.length === 0 ? (
            <View style={styles.note}>
              <Text style={styles.noteText}>
                No skills found. Add SKILL.md folders to the skills folder next to config.toml, or to LM Studio's skills
                folder.
              </Text>
            </View>
          ) : null}
          {(skills ?? []).map((k) => (
            <ToggleRow
              key={k.path}
              label={k.name}
              hint={k.description}
              value={!disabled.has(k.name)}
              onChange={(on) => {
                const next = new Set(disabled);
                if (on) next.delete(k.name);
                else next.add(k.name);
                setDraft((d) => (d ? { ...d, disabled_skills: Array.from(next).sort() } : d));
              }}
            />
          ))}
        </Group>

        <SectionTitle title="Notifications" />
        <Group>
          <ToggleRow
            label="Send push notifications"
            hint="When a task ends or needs approval. Only project name, session title and status are sent."
            value={draft.push.enabled}
            onChange={(v) => set('push', { enabled: v })}
          />
        </Group>
        <View style={{ height: dirty ? 120 : 24 }} />
      </ScrollView>

      {dirty ? (
        <View style={[styles.saveBar, { paddingBottom: Math.max(insets.bottom, space.md) }]}>
          {llamaChanged ? <Text style={styles.saveNote}>Saving reloads the model.</Text> : null}
          <View style={styles.saveRow}>
            <Button title="Discard" variant="secondary" onPress={discard} style={styles.grow} />
            <Button title="Save changes" onPress={save} loading={saving} disabled={hasInvalid} style={styles.grow} />
          </View>
        </View>
      ) : null}
    </KeyboardAvoidingView>
  );
}

const styles = StyleSheet.create({
  flex: { flex: 1, backgroundColor: colors.bg },
  grow: { flex: 1 },
  center: { flex: 1, backgroundColor: colors.bg, justifyContent: 'center', padding: space.lg },
  content: { padding: space.lg },
  note: { paddingHorizontal: space.lg, paddingVertical: space.md },
  noteText: { color: colors.textMuted, fontSize: 13, lineHeight: 18 },
  saveBar: {
    position: 'absolute',
    left: 0,
    right: 0,
    bottom: 0,
    backgroundColor: colors.surface,
    borderTopWidth: StyleSheet.hairlineWidth,
    borderTopColor: colors.border,
    paddingHorizontal: space.lg,
    paddingTop: space.md,
    gap: space.sm,
  },
  saveNote: { color: colors.warning, fontSize: 13 },
  saveRow: { flexDirection: 'row', gap: space.md },
});
