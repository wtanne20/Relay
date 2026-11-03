import { useState } from 'react';
import { ScrollView, StyleSheet, Text, View } from 'react-native';

import { parseArgs, toolDetails } from '../lib/format';
import type { ApprovalDecision, PendingApproval } from '../lib/types';
import { colors, mono, radius, space } from './theme';
import { Button } from './ui';

const QUESTION: Record<string, string> = {
  run_command: 'Run this command?',
  write_file: 'Write this file?',
  edit_file: 'Make this edit?',
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
      <Text style={styles.title}>{QUESTION[approval.name] ?? `Allow ${approval.name}?`}</Text>
      {typeof path === 'string' && approval.name !== 'run_command' ? <Text style={styles.path}>{path}</Text> : null}
      <ScrollView style={styles.details} nestedScrollEnabled>
        <Text selectable style={styles.code}>
          {approval.name === 'run_command' ? details : details.split('\n').slice(1).join('\n')}
        </Text>
      </ScrollView>
      <View style={styles.row}>
        <Button
          title="Deny"
          variant="danger"
          onPress={() => decide('deny')}
          loading={busy === 'deny'}
          disabled={busy !== null}
          style={styles.grow}
        />
        <Button
          title="Allow"
          onPress={() => decide('approve')}
          loading={busy === 'approve'}
          disabled={busy !== null}
          style={styles.grow}
        />
      </View>
      <Button
        title="Allow everything for this task"
        variant="ghost"
        onPress={() => decide('approve_all')}
        loading={busy === 'approve_all'}
        disabled={busy !== null}
      />
    </View>
  );
}

const styles = StyleSheet.create({
  card: {
    margin: space.md,
    marginBottom: 0,
    padding: space.md,
    gap: space.sm,
    borderRadius: radius.md,
    backgroundColor: colors.warningBg,
    borderWidth: 1,
    borderColor: colors.warning,
  },
  title: { color: colors.warning, fontSize: 16, fontWeight: '700' },
  path: { color: colors.text, fontSize: 14, fontWeight: '600' },
  details: { maxHeight: 160, backgroundColor: colors.code, borderRadius: radius.sm, padding: space.sm },
  code: { color: colors.text, fontFamily: mono, fontSize: 13, lineHeight: 18 },
  row: { flexDirection: 'row', gap: space.md },
  grow: { flex: 1 },
});
