import { router, useFocusEffect } from 'expo-router';
import { useCallback, useState } from 'react';
import { ActivityIndicator, KeyboardAvoidingView, Platform, Pressable, ScrollView, StyleSheet, Text, View } from 'react-native';

import { alert } from '../components/Dialogs';
import { Group, NavRow, SectionTitle, Segmented } from '../components/SettingsControls';
import { m3, mono, radius, space, type } from '../components/theme';
import { Banner, Button, Field, Icon, IconButton } from '../components/ui';
import { describeError } from '../lib/api';
import { useClient } from '../lib/connection';
import { relativeTime } from '../lib/format';
import { usePageStyle } from '../lib/layout';
import { RULE_KINDS, ruleText } from '../lib/rules';
import type { ApprovalRule, Project, RuleKind } from '../lib/types';

const KIND_HINT: Record<RuleKind, { label: string; placeholder: string; hint: string }> = {
  command: {
    label: 'Commands starting with',
    placeholder: 'git status',
    hint: "Whole words: “git status” also allows “git status -s”, but not “git push”. Commands that chain or pipe (; & | $ > and so on) are always asked about.",
  },
  files: {
    label: 'Folder in the project',
    placeholder: 'src',
    hint: 'Writing and editing any file inside this folder, relative to the project. Use . for the whole project.',
  },
  web: {
    label: 'Website',
    placeholder: 'docs.rs',
    hint: 'Reading pages on this site and its subdomains.',
  },
};

/**
 * Always-allowed actions: the rules saved from "Always allow" on an approval
 * (or added here). The agent doesn't ask about anything a rule covers.
 */
export default function ApprovalRulesScreen() {
  const page = usePageStyle();
  const client = useClient();
  const [rules, setRules] = useState<ApprovalRule[] | null>(null);
  const [projects, setProjects] = useState<Project[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState<number | 'add' | null>(null);
  const [kind, setKind] = useState<RuleKind>('command');
  const [pattern, setPattern] = useState('');
  const [projectId, setProjectId] = useState<string | null>(null);
  const [formError, setFormError] = useState<string | null>(null);

  const load = useCallback(async () => {
    if (!client) return;
    try {
      const [r, p] = await Promise.all([client.approvalRules(), client.projects()]);
      setRules(r.rules);
      setProjects(p);
      setError(null);
    } catch (e) {
      setError(describeError(e));
    }
  }, [client]);

  useFocusEffect(
    useCallback(() => {
      load();
    }, [load]),
  );

  if (!client) return null;

  const remove = (r: ApprovalRule) =>
    alert('Remove this rule?', `${ruleText(r)}. The agent asks about these again.`, [
      { text: 'Cancel', style: 'cancel' },
      {
        text: 'Remove',
        style: 'destructive',
        onPress: async () => {
          setBusy(r.id);
          try {
            await client.deleteApprovalRule(r.id);
            setRules((rs) => rs?.filter((x) => x.id !== r.id) ?? null);
          } catch (e) {
            alert('Could not remove the rule', describeError(e));
          } finally {
            setBusy(null);
          }
        },
      },
    ]);

  const add = async () => {
    if (kind === 'files' && !projectId) {
      setFormError('Pick the project the folder is in.');
      return;
    }
    setBusy('add');
    setFormError(null);
    try {
      const saved = await client.addApprovalRule({ kind, pattern, project_id: projectId });
      setRules((rs) => [saved, ...(rs ?? []).filter((x) => x.id !== saved.id)]);
      setPattern('');
    } catch (e) {
      setFormError(describeError(e));
    } finally {
      setBusy(null);
    }
  };

  const h = KIND_HINT[kind];

  return (
    <KeyboardAvoidingView style={styles.flex} behavior={Platform.OS === 'ios' ? 'padding' : undefined}>
      <ScrollView contentContainerStyle={[styles.content, page]} keyboardShouldPersistTaps="handled">
        <Text style={[type.bodyMedium, styles.intro]}>
          The agent doesn't ask before doing anything on this list. Tap “Always allow” on an approval to add to it.
          Reading files and searching the web are never asked about.
        </Text>
        {error ? <Banner message={error} actionLabel="Retry" onAction={load} /> : null}

        <SectionTitle title="Rules" />
        <Group>
          {!rules && !error ? <ActivityIndicator color={m3.primary} style={styles.pad} /> : null}
          {rules && !rules.length ? (
            <View style={styles.empty}>
              <Icon name="shield-check-outline" size={32} color={m3.outline} />
              <Text style={[type.bodyMedium, styles.muted]}>Nothing yet. The agent asks about every change.</Text>
            </View>
          ) : null}
          {rules?.map((r, i) => (
            <View key={r.id}>
              {i > 0 ? <View style={styles.divider} /> : null}
              <View style={styles.row}>
                <Icon name={RULE_KINDS.find((k) => k.value === r.kind)?.icon ?? 'check'} size={20} color={m3.onSurfaceVariant} />
                <View style={styles.grow}>
                  <Text style={type.bodyLarge}>{ruleText(r)}</Text>
                  <Text style={type.bodySmall} numberOfLines={1}>
                    {r.project_id ? (r.project_name ?? 'One project') : 'All projects'}
                    {' · '}
                    {r.uses ? `used ${r.uses} time${r.uses === 1 ? '' : 's'}, last ${relativeTime(r.last_used_at ?? r.created_at)}` : `added ${relativeTime(r.created_at)}`}
                  </Text>
                </View>
                {busy === r.id ? (
                  <ActivityIndicator color={m3.primary} style={styles.spinner} />
                ) : (
                  <IconButton icon="delete-outline" label={`Remove ${ruleText(r)}`} onPress={() => remove(r)} />
                )}
              </View>
            </View>
          ))}
        </Group>

        <SectionTitle title="Add a rule" />
        <Group>
          <Segmented
            label="Allow"
            icon="plus-circle-outline"
            value={kind}
            options={RULE_KINDS.map(({ value, label }) => ({ value, label }))}
            onChange={(v) => {
              setKind(v);
              setFormError(null);
            }}
          />
          <View style={styles.form}>
            <Field
              label={h.label}
              hint={h.hint}
              value={pattern}
              onChangeText={setPattern}
              placeholder={h.placeholder}
              style={kind === 'command' ? { fontFamily: mono } : undefined}
              onSubmitEditing={add}
            />
            <Text style={[type.bodySmall, styles.label]}>Where</Text>
            <View style={styles.chips}>
              {kind !== 'files' ? (
                <Chip label="All projects" on={projectId === null} onPress={() => setProjectId(null)} />
              ) : null}
              {projects.map((p) => (
                <Chip key={p.id} label={p.name} on={projectId === p.id} onPress={() => setProjectId(p.id)} />
              ))}
            </View>
            {formError ? <Text style={[type.bodyMedium, styles.error]}>{formError}</Text> : null}
            <Button
              title="Add rule"
              icon="plus"
              onPress={add}
              loading={busy === 'add'}
              disabled={!pattern.trim() || busy !== null}
              style={styles.addButton}
            />
          </View>
        </Group>

        <SectionTitle title="History" />
        <Group>
          <NavRow
            icon="history"
            label="Activity"
            value="Everything the agent did, and who allowed it"
            onPress={() => router.push('/activity')}
          />
        </Group>
      </ScrollView>
    </KeyboardAvoidingView>
  );
}

function Chip({ label, on, onPress }: { label: string; on: boolean; onPress: () => void }) {
  return (
    <Pressable
      onPress={onPress}
      style={[styles.chip, on && styles.chipOn]}
      accessibilityRole="radio"
      accessibilityState={{ selected: on }}
      accessibilityLabel={label}
    >
      {on ? <Icon name="check" size={16} color={m3.onSecondaryContainer} /> : null}
      <Text style={[type.labelLarge, { color: on ? m3.onSecondaryContainer : m3.onSurface }]} numberOfLines={1}>
        {label}
      </Text>
    </Pressable>
  );
}

const styles = StyleSheet.create({
  flex: { flex: 1, backgroundColor: m3.surface },
  content: { padding: space.lg, paddingBottom: 64 },
  intro: { color: m3.onSurfaceVariant, marginBottom: space.sm },
  pad: { margin: space.lg },
  empty: { alignItems: 'center', gap: space.sm, padding: space.xl },
  muted: { color: m3.onSurfaceVariant, textAlign: 'center' },
  divider: { height: StyleSheet.hairlineWidth, backgroundColor: m3.outlineVariant, marginLeft: 56 },
  row: { flexDirection: 'row', alignItems: 'center', gap: space.lg, paddingLeft: space.lg, paddingRight: space.xs, paddingVertical: space.sm, minHeight: 64 },
  grow: { flex: 1, minWidth: 0 },
  spinner: { width: 48 },
  form: { paddingHorizontal: space.lg, paddingBottom: space.lg, gap: space.sm },
  label: { color: m3.onSurfaceVariant, marginTop: space.sm },
  chips: { flexDirection: 'row', flexWrap: 'wrap', gap: space.sm },
  chip: {
    flexDirection: 'row',
    alignItems: 'center',
    gap: space.xs,
    height: 32,
    paddingHorizontal: space.md,
    borderRadius: radius.sm,
    borderWidth: 1,
    borderColor: m3.outlineVariant,
    maxWidth: 220,
  },
  chipOn: { backgroundColor: m3.secondaryContainer, borderColor: m3.secondaryContainer },
  error: { color: m3.error },
  addButton: { alignSelf: 'flex-start', marginTop: space.sm },
});
