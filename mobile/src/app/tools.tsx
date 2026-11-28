import { useFocusEffect } from 'expo-router';
import { useCallback, useState } from 'react';
import {
  ActivityIndicator,
  KeyboardAvoidingView,
  Platform,
  Pressable,
  ScrollView,
  StyleSheet,
  Switch,
  Text,
  View,
} from 'react-native';

import { alert } from '../components/Dialogs';
import { Group, SectionTitle } from '../components/SettingsControls';
import { Sheet } from '../components/Sheet';
import { m3, mono, radius, space, type } from '../components/theme';
import { Banner, Button, Field, Icon, IconButton, type IconName } from '../components/ui';
import { describeError } from '../lib/api';
import { useClient } from '../lib/connection';
import { usePageStyle } from '../lib/layout';
import { parseToolFile, pickToolFile, saveToolFile, toolFile, type ParsedToolFile } from '../lib/toolFile';
import type { BuiltinTool, CustomTool, ToolDefinition, ToolsResponse } from '../lib/types';

const TOOL_ICONS: Record<string, IconName> = {
  list_dir: 'folder-outline',
  read_file: 'file-document-outline',
  write_file: 'file-plus-outline',
  edit_file: 'file-edit-outline',
  run_command: 'console',
  web_search: 'magnify',
  fetch_url: 'web',
  search_documents: 'file-search-outline',
};

const EMPTY_TOOL: CustomTool = {
  name: '',
  description: '',
  command: '',
  parameters: [],
  enabled: true,
  timeout_seconds: 0,
};

/** A name the model can call: lowercase, digits and underscores. */
function toIdent(s: string) {
  return s
    .toLowerCase()
    .replace(/[\s-]+/g, '_')
    .replace(/[^a-z0-9_]/g, '');
}

function isPowerShell(shell: string) {
  return /powershell|pwsh/i.test(shell);
}

function shellName(shell: string) {
  if (isPowerShell(shell)) return 'PowerShell';
  if (/cmd(\.exe)?$/i.test(shell)) return 'cmd';
  return shell;
}

function Toggle({ value, onChange, label, disabled }: { value: boolean; onChange: (v: boolean) => void; label: string; disabled?: boolean }) {
  return (
    <Switch
      value={value}
      onValueChange={onChange}
      disabled={disabled}
      accessibilityLabel={label}
      trackColor={{ true: m3.primary, false: m3.surfaceContainerHighest }}
      thumbColor={value ? m3.onPrimary : m3.outline}
      {...({ activeThumbColor: m3.onPrimary } as object)}
      ios_backgroundColor={m3.surfaceContainerHighest}
    />
  );
}

/** What the model is told about a tool: its description and parameters. */
function DefinitionView({ def }: { def: ToolDefinition }) {
  const f = def.function;
  const props = Object.entries(f.parameters?.properties ?? {});
  const required = new Set(f.parameters?.required ?? []);
  return (
    <View style={styles.def}>
      <Text style={styles.defName}>{f.name}</Text>
      <Text style={[type.bodyMedium, styles.muted]}>{f.description}</Text>
      {props.length ? (
        <View style={styles.params}>
          {props.map(([name, p]) => (
            <View key={name} style={styles.paramLine}>
              <Text style={styles.paramName}>
                {name}
                {required.has(name) ? '' : '?'}
              </Text>
              <Text style={[type.bodySmall, styles.grow]}>{p.description ?? p.type ?? ''}</Text>
            </View>
          ))}
        </View>
      ) : null}
    </View>
  );
}

function BuiltinRow({
  tool,
  first,
  busy,
  onToggle,
}: {
  tool: BuiltinTool;
  first: boolean;
  busy: boolean;
  onToggle: (v: boolean) => void;
}) {
  const [open, setOpen] = useState(false);
  return (
    <View>
      {first ? null : <View style={styles.divider} />}
      <Pressable
        onPress={() => setOpen((o) => !o)}
        style={({ pressed }) => [styles.row, pressed && styles.pressed]}
        accessibilityRole="button"
        accessibilityState={{ expanded: open }}
        accessibilityLabel={`${tool.summary}, details`}
      >
        <Icon name={TOOL_ICONS[tool.name] ?? 'wrench-outline'} size={22} color={m3.onSurfaceVariant} />
        <View style={styles.grow}>
          <Text style={type.bodyLarge}>{tool.summary}</Text>
          <View style={styles.nameLine}>
            <Text style={styles.code}>{tool.name}</Text>
            <Icon name={open ? 'chevron-up' : 'chevron-down'} size={16} color={m3.outline} />
          </View>
        </View>
        {busy ? <ActivityIndicator color={m3.primary} /> : null}
        <Toggle value={tool.enabled} onChange={onToggle} label={tool.summary} disabled={busy} />
      </Pressable>
      {open ? (
        <View style={styles.details}>
          <Text style={[type.labelMedium, styles.muted]}>What the agent is told</Text>
          {tool.definitions.map((d) => (
            <DefinitionView key={d.function.name} def={d} />
          ))}
        </View>
      ) : null}
    </View>
  );
}

function CustomRow({
  tool,
  first,
  busy,
  onToggle,
  onEdit,
}: {
  tool: CustomTool;
  first: boolean;
  busy: boolean;
  onToggle: (v: boolean) => void;
  onEdit: () => void;
}) {
  return (
    <View>
      {first ? null : <View style={styles.divider} />}
      <Pressable
        onPress={onEdit}
        style={({ pressed }) => [styles.row, pressed && styles.pressed]}
        accessibilityRole="button"
        accessibilityLabel={`Edit ${tool.name}`}
      >
        <Icon name="wrench-outline" size={22} color={m3.onSurfaceVariant} />
        <View style={styles.grow}>
          <Text style={[type.bodyLarge, styles.codeTitle]}>{tool.name}</Text>
          <Text style={type.bodyMedium} numberOfLines={2}>
            {tool.description}
          </Text>
          <Text style={[styles.code, styles.commandLine]} numberOfLines={1} ellipsizeMode="tail">
            {tool.command}
          </Text>
        </View>
        {busy ? <ActivityIndicator color={m3.primary} /> : null}
        <Toggle value={tool.enabled} onChange={onToggle} label={`${tool.name} on`} disabled={busy} />
      </Pressable>
    </View>
  );
}

/** The command as it would run, with each parameter shown as a quoted sample. */
function preview(tool: CustomTool, shell: string) {
  let cmd = tool.command;
  for (const p of tool.parameters) {
    if (!p.name) continue;
    cmd = cmd.split(`{${p.name}}`).join(isPowerShell(shell) || !/cmd/i.test(shell) ? `'<${p.name}>'` : `"<${p.name}>"`);
  }
  return cmd;
}

/** Add or edit one of the user's tools. */
function ToolEditor({
  visible,
  initial,
  editing,
  shell,
  defaultTimeout,
  onClose,
  onSave,
  onDelete,
}: {
  visible: boolean;
  initial: CustomTool;
  /** The tool's name when editing an existing one. */
  editing: string | null;
  shell: string;
  defaultTimeout: number;
  onClose: () => void;
  onSave: (t: CustomTool) => Promise<void>;
  onDelete: () => void;
}) {
  const [tool, setTool] = useState<CustomTool>(initial);
  const [timeout, setTimeoutText] = useState(initial.timeout_seconds ? String(initial.timeout_seconds) : '');
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  const set = (patch: Partial<CustomTool>) => setTool((t) => ({ ...t, ...patch }));
  const setParam = (i: number, patch: Partial<CustomTool['parameters'][number]>) =>
    setTool((t) => ({ ...t, parameters: t.parameters.map((p, j) => (j === i ? { ...p, ...patch } : p)) }));

  const addParam = () =>
    setTool((t) => ({ ...t, parameters: [...t.parameters, { name: '', description: '', required: true }] }));

  const missing = tool.parameters.filter((p) => p.name && !tool.command.includes(`{${p.name}}`)).map((p) => p.name);
  const timeoutNum = timeout.trim() ? Number(timeout) : 0;
  const timeoutBad = !Number.isInteger(timeoutNum) || timeoutNum < 0 || timeoutNum > 3600;
  const ready = !!tool.name && !!tool.description.trim() && !!tool.command.trim() && !timeoutBad;

  const save = async () => {
    setSaving(true);
    setError(null);
    try {
      await onSave({ ...tool, timeout_seconds: timeoutNum });
    } catch (e) {
      setError(describeError(e));
    } finally {
      setSaving(false);
    }
  };

  return (
    <Sheet visible={visible} onClose={onClose} title={editing ? 'Edit tool' : 'Add a tool'}>
      <ScrollView style={styles.sheetScroll} contentContainerStyle={styles.sheetBody} keyboardShouldPersistTaps="handled">
        <Field
          label="Name"
          hint="What the agent calls it. Lowercase letters, numbers and _."
          value={tool.name}
          onChangeText={(v) => set({ name: toIdent(v) })}
          placeholder="git_log"
          style={{ fontFamily: mono }}
        />
        <Field
          label="Description"
          hint="Tell the agent what it does and when to use it. This is all it knows about the tool."
          value={tool.description}
          onChangeText={(v) => set({ description: v })}
          placeholder="Show the latest commits in the project. Use it when asked what changed recently."
          multiline
        />
        <Field
          label={`Command (${shellName(shell)})`}
          hint="Runs in the project folder. Put {name} where a parameter's value goes. Values are always quoted, so they can't add commands of their own."
          value={tool.command}
          onChangeText={(v) => set({ command: v })}
          placeholder="git log --oneline -n {count}"
          multiline
          style={{ fontFamily: mono }}
        />

        <View style={styles.paramsHead}>
          <Text style={type.titleSmall}>Parameters</Text>
          <Button title="Add" icon="plus" variant="text" onPress={addParam} disabled={tool.parameters.length >= 16} />
        </View>
        {!tool.parameters.length ? (
          <Text style={[type.bodySmall, styles.muted]}>
            None. Add one for each value the agent should fill in, like a file name or a count.
          </Text>
        ) : null}
        {tool.parameters.map((p, i) => (
          <View key={i} style={styles.paramCard}>
            <View style={styles.paramTop}>
              <View style={styles.grow}>
                <Field
                  label="Parameter name"
                  value={p.name}
                  onChangeText={(v) => setParam(i, { name: toIdent(v) })}
                  placeholder="count"
                  style={{ fontFamily: mono }}
                />
              </View>
              <IconButton
                icon="delete-outline"
                label={`Remove parameter ${p.name || i + 1}`}
                onPress={() => setTool((t) => ({ ...t, parameters: t.parameters.filter((_, j) => j !== i) }))}
              />
            </View>
            <Field
              label="What it is"
              value={p.description}
              onChangeText={(v) => setParam(i, { description: v })}
              placeholder="How many commits to show"
            />
            <Pressable
              onPress={() => setParam(i, { required: !p.required })}
              style={styles.requiredRow}
              accessibilityRole="switch"
              accessibilityState={{ checked: p.required }}
              accessibilityLabel="Required"
            >
              <Text style={[type.bodyMedium, styles.grow]}>
                {p.required ? 'Required' : 'Optional: left out of the command when not given'}
              </Text>
              <Toggle value={p.required} onChange={(v) => setParam(i, { required: v })} label="Required" />
            </Pressable>
          </View>
        ))}
        {missing.length ? (
          <Text style={[type.bodySmall, styles.warn]}>
            The command doesn&apos;t use {missing.map((n) => `{${n}}`).join(', ')} yet.
          </Text>
        ) : null}

        <Field
          label="Timeout in seconds"
          hint={`Leave empty to use the agent's command timeout (${defaultTimeout} s).`}
          value={timeout}
          onChangeText={(v) => setTimeoutText(v.replace(/[^0-9]/g, ''))}
          placeholder={String(defaultTimeout)}
          keyboardType="number-pad"
        />

        {tool.command.trim() ? (
          <View style={styles.previewBox}>
            <Text style={[type.labelMedium, styles.muted]}>Runs as</Text>
            <Text selectable style={styles.previewText}>
              {preview(tool, shell)}
            </Text>
          </View>
        ) : null}

        <Text style={[type.bodySmall, styles.muted]}>
          When approvals are on for commands, the agent asks before each use and shows the exact command.
        </Text>

        {error ? <Banner message={error} /> : null}
      </ScrollView>
      <View style={styles.sheetButtons}>
        {editing ? (
          <IconButton icon="delete-outline" label={`Delete ${editing}`} onPress={onDelete} color={m3.error} />
        ) : null}
        <View style={styles.grow} />
        <Button title="Cancel" variant="text" onPress={onClose} />
        <Button title="Save" icon="check" onPress={save} loading={saving} disabled={!ready || saving} />
      </View>
    </Sheet>
  );
}

/**
 * Shows every tool in a file, with its full command, before any of it is
 * added. Imported tools come in turned off: the user turns each one on
 * once they've read what it runs.
 */
function ImportReview({
  file,
  existing,
  shell,
  onClose,
  onImport,
}: {
  file: ParsedToolFile;
  /** Names of the user's current tools. */
  existing: string[];
  shell: string;
  onClose: () => void;
  onImport: (tools: CustomTool[]) => Promise<void>;
}) {
  const [picked, setPicked] = useState<boolean[]>(() => file.tools.map((t) => !existing.includes(t.name)));
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const chosen = file.tools.filter((_, i) => picked[i]);
  const otherShell = file.shell && shellName(file.shell) !== shellName(shell) ? file.shell : null;

  const go = async () => {
    setSaving(true);
    setError(null);
    try {
      await onImport(chosen);
    } catch (e) {
      setError(describeError(e));
    } finally {
      setSaving(false);
    }
  };

  return (
    <Sheet visible onClose={onClose} title="Review tools to import">
      <ScrollView style={styles.sheetScroll} contentContainerStyle={styles.sheetBody}>
        <Text style={[type.bodyMedium, styles.muted]}>
          These tools run commands on this computer. Read each command before you import it. They come in turned off,
          so nothing runs until you turn a tool on.
        </Text>
        {otherShell ? (
          <Banner
            kind="warning"
            message={`These were written for ${shellName(otherShell)}, but commands here run in ${shellName(shell)}. They may need changes to work.`}
          />
        ) : null}
        {file.tools.map((t, i) => {
          const replaces = existing.includes(t.name);
          return (
            <View key={i} style={styles.importCard}>
              <Pressable
                onPress={() => setPicked((p) => p.map((v, j) => (j === i ? !v : v)))}
                style={styles.importHead}
                accessibilityRole="checkbox"
                accessibilityState={{ checked: picked[i] }}
                accessibilityLabel={`Import ${t.name || 'unnamed tool'}`}
              >
                <Icon
                  name={picked[i] ? 'checkbox-marked' : 'checkbox-blank-outline'}
                  size={22}
                  color={picked[i] ? m3.primary : m3.outline}
                />
                <Text style={[type.bodyLarge, styles.codeTitle, styles.grow]}>{t.name || '(no name)'}</Text>
              </Pressable>
              {replaces ? (
                <Text style={[type.bodySmall, styles.warn]}>
                  You already have a tool called {t.name}. Importing this one replaces it.
                </Text>
              ) : null}
              {t.description ? <Text style={type.bodyMedium}>{t.description}</Text> : null}
              <View style={styles.previewBox}>
                <Text style={[type.labelMedium, styles.muted]}>Runs</Text>
                <Text selectable style={styles.previewText}>
                  {t.command}
                </Text>
              </View>
              {t.parameters.length ? (
                <View style={styles.params}>
                  {t.parameters.map((p, j) => (
                    <View key={j} style={styles.paramLine}>
                      <Text style={styles.paramName}>
                        {`{${p.name}}`}
                        {p.required ? '' : '?'}
                      </Text>
                      <Text style={[type.bodySmall, styles.grow]}>{p.description}</Text>
                    </View>
                  ))}
                </View>
              ) : null}
              {t.timeout_seconds ? (
                <Text style={[type.bodySmall, styles.muted]}>Stops after {t.timeout_seconds} s.</Text>
              ) : null}
            </View>
          );
        })}
        {error ? <Banner message={error} /> : null}
      </ScrollView>
      <View style={styles.sheetButtons}>
        <View style={styles.grow} />
        <Button title="Cancel" variant="text" onPress={onClose} />
        <Button
          title={chosen.length === 1 ? 'Import 1 tool' : `Import ${chosen.length} tools`}
          icon="tray-arrow-down"
          onPress={go}
          loading={saving}
          disabled={!chosen.length || saving}
        />
      </View>
    </Sheet>
  );
}

/**
 * Every tool the agent can use: the built-in ones (turn them on or off and
 * see what the agent is told about each) and tools of your own.
 */
export default function ToolsScreen() {
  const page = usePageStyle();
  const client = useClient();
  const [data, setData] = useState<ToolsResponse | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  // The open editor: null = closed; editing = the tool's current name.
  const [editor, setEditor] = useState<{ tool: CustomTool; editing: string | null; key: number } | null>(null);
  const [importing, setImporting] = useState<ParsedToolFile | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  const load = useCallback(async () => {
    if (!client) return;
    try {
      setData(await client.tools());
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

  const toggleBuiltin = async (t: BuiltinTool, v: boolean) => {
    setBusy(t.name);
    try {
      await client.patchSettings({ agent: { tools: { [t.name]: v } } });
      setData((d) => (d ? { ...d, builtin: d.builtin.map((b) => (b.name === t.name ? { ...b, enabled: v } : b)) } : d));
    } catch (e) {
      alert('Could not change the tool', describeError(e));
    } finally {
      setBusy(null);
    }
  };

  const toggleCustom = async (t: CustomTool, v: boolean) => {
    setBusy(`custom:${t.name}`);
    try {
      setData(await client.updateTool(t.name, { ...t, enabled: v }));
    } catch (e) {
      alert('Could not change the tool', describeError(e));
    } finally {
      setBusy(null);
    }
  };

  const open = (tool: CustomTool, editing: string | null) => setEditor({ tool, editing, key: Date.now() });

  const save = async (t: CustomTool) => {
    const next = editor?.editing ? await client.updateTool(editor.editing, t) : await client.addTool(t);
    setData(next);
    setEditor(null);
  };

  const exportTools = async () => {
    if (!data?.custom.length) return;
    try {
      if (await saveToolFile(toolFile(data.custom, data.shell))) {
        const n = data.custom.length;
        setNotice(`Exported ${n === 1 ? '1 tool' : `${n} tools`} to relay-tools.json.`);
      }
    } catch (e) {
      alert('Could not export your tools', describeError(e));
    }
  };

  const importFile = async () => {
    setNotice(null);
    try {
      const text = await pickToolFile();
      if (text != null) setImporting(parseToolFile(text));
    } catch (e) {
      alert('Could not read that file', describeError(e));
    }
  };

  const doImport = async (tools: CustomTool[]) => {
    const next = await client.importTools(tools);
    setData(next);
    setImporting(null);
    const n = next.imported.length;
    setNotice(
      `Imported ${n === 1 ? '1 tool' : `${n} tools`}, turned off. Turn each one on once you're happy with what it runs.`,
    );
  };

  const remove = () => {
    const name = editor?.editing;
    if (!name) return;
    alert(`Delete ${name}?`, 'The agent can no longer use it. This cannot be undone.', [
      { text: 'Cancel', style: 'cancel' },
      {
        text: 'Delete',
        style: 'destructive',
        onPress: async () => {
          try {
            setData(await client.deleteTool(name));
            setEditor(null);
          } catch (e) {
            alert('Could not delete the tool', describeError(e));
          }
        },
      },
    ]);
  };

  return (
    <KeyboardAvoidingView style={styles.flex} behavior={Platform.OS === 'ios' ? 'padding' : undefined}>
      <ScrollView contentContainerStyle={[styles.content, page]} keyboardShouldPersistTaps="handled">
        <Text style={[type.bodyMedium, styles.intro]}>
          The agent picks from these tools while it works. Turn off any it shouldn&apos;t use, or add your own: a
          command with a few values the agent fills in.
        </Text>
        {error ? <Banner message={error} actionLabel="Retry" onAction={load} /> : null}
        {notice ? <Banner kind="info" message={notice} actionLabel="OK" onAction={() => setNotice(null)} /> : null}

        <SectionTitle title="Your tools" />
        <Group>
          {!data && !error ? <ActivityIndicator color={m3.primary} style={styles.pad} /> : null}
          {data && !data.custom.length ? (
            <View style={styles.empty}>
              <Icon name="wrench-outline" size={32} color={m3.outline} />
              <Text style={[type.bodyMedium, styles.muted, styles.center]}>
                No tools of your own yet. Add one to give the agent a shortcut, like running your tests or a script.
              </Text>
            </View>
          ) : null}
          {data?.custom.map((t, i) => (
            <CustomRow
              key={t.name}
              tool={t}
              first={i === 0}
              busy={busy === `custom:${t.name}`}
              onToggle={(v) => toggleCustom(t, v)}
              onEdit={() => open(t, t.name)}
            />
          ))}
          {data ? (
            <View style={styles.addRow}>
              <Button title="Add a tool" icon="plus" variant="tonal" onPress={() => open(EMPTY_TOOL, null)} />
              <Button title="Import" icon="tray-arrow-down" variant="text" onPress={importFile} />
              {data.custom.length ? (
                <Button title="Export" icon="tray-arrow-up" variant="text" onPress={exportTools} />
              ) : null}
            </View>
          ) : null}
        </Group>
        {data?.custom.length ? (
          <Text style={[type.bodySmall, styles.footnote]}>
            Your tools work in projects (on this PC or a linked computer), not in plain chats.
          </Text>
        ) : null}

        <SectionTitle title="Built-in tools" />
        <Group>
          {!data && !error ? <ActivityIndicator color={m3.primary} style={styles.pad} /> : null}
          {data?.builtin.map((t, i) => (
            <BuiltinRow
              key={t.name}
              tool={t}
              first={i === 0}
              busy={busy === t.name}
              onToggle={(v) => toggleBuiltin(t, v)}
            />
          ))}
        </Group>
        <Text style={[type.bodySmall, styles.footnote]}>Tap a tool to see what the agent is told about it.</Text>
      </ScrollView>

      {editor && data ? (
        <ToolEditor
          key={editor.key}
          visible
          initial={editor.tool}
          editing={editor.editing}
          shell={data.shell}
          defaultTimeout={data.default_timeout}
          onClose={() => setEditor(null)}
          onSave={save}
          onDelete={remove}
        />
      ) : null}
      {importing && data ? (
        <ImportReview
          file={importing}
          existing={data.custom.map((t) => t.name)}
          shell={data.shell}
          onClose={() => setImporting(null)}
          onImport={doImport}
        />
      ) : null}
    </KeyboardAvoidingView>
  );
}

const styles = StyleSheet.create({
  flex: { flex: 1, backgroundColor: m3.surface },
  content: { padding: space.lg, paddingBottom: 64 },
  intro: { color: m3.onSurfaceVariant, marginBottom: space.sm },
  footnote: { color: m3.onSurfaceVariant, marginTop: space.sm, paddingHorizontal: space.lg },
  pad: { margin: space.lg },
  empty: { alignItems: 'center', gap: space.sm, padding: space.xl },
  muted: { color: m3.onSurfaceVariant },
  warn: { color: m3.warning },
  center: { textAlign: 'center' },
  grow: { flex: 1, minWidth: 0 },
  divider: { height: StyleSheet.hairlineWidth, backgroundColor: m3.outlineVariant, marginLeft: 56 },
  row: {
    flexDirection: 'row',
    alignItems: 'center',
    gap: space.lg,
    minHeight: 64,
    paddingHorizontal: space.lg,
    paddingVertical: space.sm,
  },
  pressed: { backgroundColor: m3.pressed },
  nameLine: { flexDirection: 'row', alignItems: 'center', gap: space.xs },
  code: { fontFamily: mono, fontSize: 13, color: m3.onSurfaceVariant },
  codeTitle: { fontFamily: mono },
  commandLine: { marginTop: 2, color: m3.outline },
  addRow: { flexDirection: 'row', flexWrap: 'wrap', gap: space.sm, paddingHorizontal: space.lg, paddingVertical: space.md },
  importCard: {
    borderWidth: 1,
    borderColor: m3.outlineVariant,
    borderRadius: radius.lg,
    padding: space.md,
    gap: space.sm,
  },
  importHead: { flexDirection: 'row', alignItems: 'center', gap: space.sm },
  details: { paddingLeft: 56, paddingRight: space.lg, paddingBottom: space.lg, gap: space.sm },
  def: {
    backgroundColor: m3.surfaceContainer,
    borderRadius: radius.md,
    padding: space.md,
    gap: space.xs,
  },
  defName: { fontFamily: mono, fontSize: 13, color: m3.primary },
  params: { gap: 4, marginTop: space.xs },
  paramLine: { flexDirection: 'row', gap: space.sm },
  paramName: { fontFamily: mono, fontSize: 12, color: m3.onSurface, minWidth: 128 },
  sheetScroll: { maxHeight: 520, flexGrow: 0 },
  sheetBody: { gap: space.md, paddingBottom: space.md },
  paramsHead: { flexDirection: 'row', alignItems: 'center', justifyContent: 'space-between', marginTop: space.sm },
  paramCard: {
    borderWidth: 1,
    borderColor: m3.outlineVariant,
    borderRadius: radius.lg,
    padding: space.md,
    gap: space.sm,
  },
  paramTop: { flexDirection: 'row', alignItems: 'flex-start', gap: space.xs },
  requiredRow: { flexDirection: 'row', alignItems: 'center', gap: space.sm },
  previewBox: {
    backgroundColor: m3.surfaceContainerHighest,
    borderRadius: radius.md,
    padding: space.md,
    gap: space.xs,
  },
  previewText: { fontFamily: mono, fontSize: 13, color: m3.onSurface },
  sheetButtons: { flexDirection: 'row', alignItems: 'center', gap: space.sm, paddingTop: space.md },
});
