import { useState } from 'react';
import { ScrollView, StyleSheet, Text, View } from 'react-native';

import { parseArgs, toolDetails } from '../lib/format';
import type { ApprovalDecision, PendingApproval } from '../lib/types';
import { m3, mono, radius, space, type } from './theme';
import { Button, Icon } from './ui';

const QUESTION: Record<string, string> = {
  run_command: 'Run this command?',
  write_file: 'Write this file?',
  edit_file: 'Make this edit?',
  fetch_url: 'Open this web page?',
};

/** Shown above the composer while the agent waits for permission. */
export function ApprovalCard({
  approval,
  onDecide,
}: {
  approval: PendingApproval;
  onDecide: (decision: ApprovalDecision) => Promise<void>;
}) {
  const [busy, setBusy] = useState<ApprovalDecision | null>(null);
  const call = { id: approval.tool_call_id, name: approval.name, arguments: approval.arguments };
  const details = toolDetails(call);
  const path = parseArgs(call).path;

  const decide = async (d: ApprovalDecision) => {
    setBusy(d);
    try {
      await onDecide(d);
    } finally {
      setBusy(null);
    }
  };

  return (
    <View style={styles.card} accessibilityRole="alert">
      <View style={styles.head}>
        <View style={styles.icon}>
          <Icon name="shield-alert-outline" size={22} color={m3.warningContainer} />
        </View>
        <View style={styles.grow}>
          <Text style={[type.titleMedium, { color: m3.onWarningContainer }]}>
            {QUESTION[approval.name] ?? `Allow ${approval.name}?`}
          </Text>
          {typeof path === 'string' && approval.name !== 'run_command' ? (
            <Text style={[type.bodyMedium, styles.path]} numberOfLines={1} ellipsizeMode="middle">
              {path}
            </Text>
          ) : (
            <Text style={[type.bodyMedium, styles.path]}>The agent is waiting for you</Text>
          )}
        </View>
      </View>
      <ScrollView style={styles.details} nestedScrollEnabled>
        <Text selectable style={styles.code}>
          {approval.name === 'run_command' ? details : details.split('\n').slice(1).join('\n')}
        </Text>
      </ScrollView>
      <View style={styles.row}>
        <Button
          title="Deny"
          icon="close"
          variant="outlined"
          onPress={() => decide('deny')}
          loading={busy === 'deny'}
          disabled={busy !== null}
          style={styles.grow}
        />
        <Button
          title="Allow"
          icon="check"
          onPress={() => decide('approve')}
          loading={busy === 'approve'}
          disabled={busy !== null}
          style={styles.grow}
        />
      </View>
      {approval.suggested_rule ? (
        <Button
          title={`Always allow: ${lowerFirst(approval.suggested_rule.label ?? approval.suggested_rule.pattern)}`}
          icon="check-all"
          variant="tonal"
          onPress={() => decide('always_allow')}
          loading={busy === 'always_allow'}
          disabled={busy !== null}
        />
      ) : null}
      <Button
        title="Allow everything for this task"
        variant="text"
        onPress={() => decide('approve_all')}
        loading={busy === 'approve_all'}
        disabled={busy !== null}
      />
    </View>
  );
}

const lowerFirst = (s: string) => {
  const t = s.length > 60 ? `${s.slice(0, 59)}…` : s;
  return t ? t[0].toLowerCase() + t.slice(1) : t;
};

const styles = StyleSheet.create({
  card: {
    margin: space.md,
    marginBottom: 0,
    padding: space.lg,
    paddingBottom: space.sm,
    gap: space.md,
    borderRadius: radius.xl,
    backgroundColor: m3.warningContainer,
  },
  head: { flexDirection: 'row', alignItems: 'center', gap: space.md },
  icon: {
    width: 40,
    height: 40,
    borderRadius: 20,
    backgroundColor: m3.warning,
    alignItems: 'center',
    justifyContent: 'center',
  },
  path: { color: m3.onWarningContainer, opacity: 0.8 },
  details: { maxHeight: 160, backgroundColor: m3.code, borderRadius: radius.md, padding: space.md },
  code: { color: m3.onSurface, fontFamily: mono, fontSize: 13, lineHeight: 19 },
  row: { flexDirection: 'row', gap: space.md },
  grow: { flex: 1, minWidth: 0 },
});
