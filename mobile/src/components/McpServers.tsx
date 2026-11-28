import { router } from 'expo-router';
import { useCallback, useEffect, useRef, useState } from 'react';
import { ActivityIndicator, Pressable, ScrollView, StyleSheet, Switch, Text, View } from 'react-native';

import { describeError, type RelayClient } from '../lib/api';
import type { McpServer, McpServerView, McpTestResult, McpTool } from '../lib/types';
import { alert } from './Dialogs';
import { Group, SectionTitle } from './SettingsControls';
import { Sheet } from './Sheet';
import { m3, mono, radius, space, type } from './theme';
import { Banner, Button, Field, Icon, IconButton } from './ui';

const EMPTY: McpServer = {
  name: '',
  description: '',
  enabled: true,
  command: '',
  args: [],
  env: {},
  cwd: '',
  url: '',
  headers: {},
  timeout_seconds: 0,
  disabled_tools: [],
  trusted: false,
};

/** A server name: lowercase letter first, a-z, 0-9 and single _. */
function toName(s: string) {
  return s
    .toLowerCase()
    .replace(/[\s.-]+/g, '_')
    .replace(/[^a-z0-9_]/g, '')
    .replace(/_+/g, '_')
    .replace(/^[^a-z]+/, '');
}

const linesToList = (s: string) =>
  s
    .split('\n')
    .map((l) => l.trim())
    .filter(Boolean);

/** `KEY=value` (env) or `Name: value` (headers), one per line. */
function linesToMap(s: string, sep: '=' | ':') {
  const out: Record<string, string> = {};
  for (const line of s.split('\n')) {
    const i = line.indexOf(sep);
    if (i <= 0) continue;
    out[line.slice(0, i).trim()] = line.slice(i + 1).trim();
  }
  return out;
}

const mapToLines = (m: Record<string, string>, sep: string) =>
  Object.entries(m)
    .map(([k, v]) => `${k}${sep}${v}`)
    .join('\n');

/** Strips the status the server adds, leaving what's saved. */
function toConfig(v: McpServerView): McpServer {
  const { status: _s, transport: _t, ...cfg } = v;
  return cfg;
}

function stateLine(s: McpServerView): { text: string; color: string } {
  if (!s.enabled) return { text: 'Off', color: m3.outline };
  const n = s.status.tools.length;
  switch (s.status.state) {
    case 'ready':
      return { text: `Connected · ${n} ${n === 1 ? 'tool' : 'tools'}`, color: m3.success };
    case 'starting':
      return { text: 'Connecting…', color: m3.onSurfaceVariant };
    case 'error':
      return { text: "Couldn't connect", color: m3.error };
    default:
      return { text: 'Off', color: m3.outline };
  }
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

function ServerRow({
  server,
  first,
  busy,
  onToggle,
  onOpen,
}: {
  server: McpServerView;
  first: boolean;
  busy: boolean;
  onToggle: (v: boolean) => void;
  onOpen: () => void;
}) {
  const st = stateLine(server);
  return (
    <View>
      {first ? null : <View style={styles.divider} />}
      <Pressable
        onPress={onOpen}
        style={({ pressed }) => [styles.row, pressed && styles.pressed]}
        accessibilityRole="button"
        accessibilityLabel={`${server.name}, ${st.text}. Details`}
      >
        <Icon name={server.transport === 'http' ? 'cloud-outline' : 'puzzle-outline'} size={22} color={m3.onSurfaceVariant} />
        <View style={styles.grow}>
          <Text style={[type.bodyLarge, styles.mono]}>{server.name}</Text>
          {server.description ? (
            <Text style={type.bodyMedium} numberOfLines={2}>
              {server.description}
            </Text>
          ) : null}
          <View style={styles.stateLine}>
            {server.enabled && server.status.state === 'starting' ? (
              <ActivityIndicator size="small" color={m3.primary} />
            ) : (
              <View style={[styles.dot, { backgroundColor: st.color }]} />
            )}
            <Text style={[type.bodySmall, { color: st.color }]}>{st.text}</Text>
          </View>
        </View>
        {busy ? <ActivityIndicator color={m3.primary} /> : null}
        <Toggle value={server.enabled} onChange={onToggle} label={`${server.name} on`} disabled={busy} />
      </Pressable>
    </View>
  );
}

function ToolLine({
  tool,
  enabled,
  onToggle,
}: {
  tool: McpTool;
  enabled: boolean;
  onToggle?: (v: boolean) => void;
}) {
  const [open, setOpen] = useState(false);
  const props = Object.entries(tool.input_schema?.properties ?? {});
  const required = new Set(tool.input_schema?.required ?? []);
  return (
    <View style={styles.toolBox}>
      <Pressable
        onPress={() => setOpen((o) => !o)}
        style={styles.toolHead}
        accessibilityRole="button"
        accessibilityState={{ expanded: open }}
        accessibilityLabel={`${tool.name}, details`}
      >
        <View style={styles.grow}>
          <Text style={styles.toolName}>{tool.title || tool.name}</Text>
          <Text style={[type.bodySmall, styles.muted]} numberOfLines={open ? undefined : 2}>
            {tool.description || tool.exposed_name}
          </Text>
          <Text style={[type.labelMedium, styles.muted]}>
            {tool.read_only ? 'Only reads' : tool.destructive ? 'May change or delete things' : 'Makes changes'}
          </Text>
        </View>
        {onToggle ? <Toggle value={enabled} onChange={onToggle} label={`${tool.name} on`} /> : null}
      </Pressable>
      {open ? (
        <View style={styles.params}>
          <Text style={styles.code}>{tool.exposed_name}</Text>
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

/** A server's state and tools: turn tools off, trust it, restart, edit. */
function ServerSheet({
  server,
  onClose,
  onChange,
  onEdit,
  onRestart,
}: {
  server: McpServerView;
  onClose: () => void;
  onChange: (s: McpServer) => Promise<void>;
  onEdit: () => void;
  onRestart: () => Promise<void>;
}) {
  const [restarting, setRestarting] = useState(false);
  const cfg = toConfig(server);
  const toggleTool = (name: string, on: boolean) => {
    const disabled = on ? cfg.disabled_tools.filter((t) => t !== name) : [...cfg.disabled_tools, name];
    onChange({ ...cfg, disabled_tools: disabled }).catch((e) => alert('Could not change the tool', describeError(e)));
  };
  const restart = async () => {
    setRestarting(true);
    try {
      await onRestart();
    } finally {
      setRestarting(false);
    }
  };
  const st = stateLine(server);
  const info = server.status.server_info;
  return (
    <Sheet visible onClose={onClose} title={server.name}>
      <ScrollView style={styles.sheetScroll} contentContainerStyle={styles.sheetBody} keyboardShouldPersistTaps="handled">
        <View style={styles.stateLine}>
          <View style={[styles.dot, { backgroundColor: st.color }]} />
          <Text style={[type.bodyMedium, { color: st.color }]}>{st.text}</Text>
          {info?.name ? (
            <Text style={[type.bodySmall, styles.muted]}>
              {' '}
              · {info.name}
              {info.version ? ` ${info.version}` : ''}
            </Text>
          ) : null}
        </View>
        <Text selectable style={[styles.code, styles.launch]} numberOfLines={3}>
          {server.transport === 'http' ? server.url : [server.command, ...server.args].join(' ')}
        </Text>
        {server.status.state === 'error' && server.status.error ? (
          <ScrollView style={styles.errorBox} nestedScrollEnabled>
            <Text selectable style={[styles.code, { color: m3.onErrorContainer }]}>
              {server.status.error}
            </Text>
          </ScrollView>
        ) : null}

        <Pressable
          onPress={() => onChange({ ...cfg, trusted: !cfg.trusted }).catch((e) => alert('Could not save', describeError(e)))}
          style={styles.trustRow}
          accessibilityRole="switch"
          accessibilityState={{ checked: cfg.trusted }}
          accessibilityLabel="Run its tools without asking"
        >
          <View style={styles.grow}>
            <Text style={type.bodyLarge}>Run its tools without asking</Text>
            <Text style={[type.bodySmall, styles.muted]}>
              Off: with approvals on, the agent asks before each tool that can change something. Tools the server marks as
              read-only never ask.
            </Text>
          </View>
          <Toggle value={cfg.trusted} onChange={(v) => onChange({ ...cfg, trusted: v })} label="Trusted" />
        </Pressable>

        {server.status.tools.length ? (
          <>
            <Text style={type.titleSmall}>Tools</Text>
            {server.status.tools.map((t) => (
              <ToolLine
                key={t.name}
                tool={t}
                enabled={!cfg.disabled_tools.includes(t.name)}
                onToggle={(v) => toggleTool(t.name, v)}
              />
            ))}
          </>
        ) : server.enabled && server.status.state === 'ready' ? (
          <Text style={[type.bodyMedium, styles.muted]}>This server has no tools.</Text>
        ) : null}
      </ScrollView>
      <View style={styles.sheetButtons}>
        <Button title="Edit" icon="pencil-outline" variant="text" onPress={onEdit} />
        <View style={styles.grow} />
        {server.enabled ? (
          <Button title="Restart" icon="restart" variant="tonal" onPress={restart} loading={restarting} disabled={restarting} />
        ) : null}
        <Button title="Done" onPress={onClose} />
      </View>
    </Sheet>
  );
}

/** Add or edit a server's settings, and check it connects. */
function ServerEditor({
  client,
  initial,
  editing,
  onClose,
  onSave,
  onDelete,
}: {
  client: RelayClient;
  initial: McpServer;
  editing: string | null;
  onClose: () => void;
  onSave: (s: McpServer) => Promise<void>;
  onDelete: () => void;
}) {
  const [s, setS] = useState<McpServer>(initial);
  const [kind, setKind] = useState<'stdio' | 'http'>(initial.url ? 'http' : 'stdio');
  const [args, setArgs] = useState(initial.args.join('\n'));
  const [env, setEnv] = useState(mapToLines(initial.env, '='));
  const [headers, setHeaders] = useState(mapToLines(initial.headers, ': '));
  const [timeout, setTimeoutText] = useState(initial.timeout_seconds ? String(initial.timeout_seconds) : '');
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [testing, setTesting] = useState(false);
  const [test, setTest] = useState<McpTestResult | null>(null);

  const set = (patch: Partial<McpServer>) => setS((x) => ({ ...x, ...patch }));
  const timeoutNum = timeout.trim() ? Number(timeout) : 0;
  const built = (): McpServer => ({
    ...s,
    command: kind === 'stdio' ? s.command.trim() : '',
    args: kind === 'stdio' ? linesToList(args) : [],
    env: kind === 'stdio' ? linesToMap(env, '=') : {},
    url: kind === 'http' ? s.url.trim() : '',
    headers: kind === 'http' ? linesToMap(headers, ':') : {},
    timeout_seconds: timeoutNum,
  });
  const ready =
    !!s.name && (kind === 'stdio' ? !!s.command.trim() : /^https?:\/\/./.test(s.url.trim())) && timeoutNum <= 3600;

  const check = async () => {
    setTesting(true);
    setTest(null);
    setError(null);
    try {
      setTest(await client.testMcpServer(built()));
    } catch (e) {
      setError(describeError(e));
    } finally {
      setTesting(false);
    }
  };

  const save = async () => {
    setSaving(true);
    setError(null);
    try {
      await onSave(built());
    } catch (e) {
      setError(describeError(e));
    } finally {
      setSaving(false);
    }
  };

  return (
    <Sheet visible onClose={onClose} title={editing ? `Edit ${editing}` : 'Add an MCP server'}>
      <ScrollView style={styles.sheetScroll} contentContainerStyle={styles.sheetBody} keyboardShouldPersistTaps="handled">
        <Field
          label="Name"
          hint="Short, like github. The agent sees its tools as name__tool."
          value={s.name}
          onChangeText={(v) => set({ name: toName(v) })}
          placeholder="github"
          style={{ fontFamily: mono }}
        />
        <View style={styles.segments} accessibilityRole="radiogroup">
          {(
            [
              ['stdio', 'Program on this PC'],
              ['http', 'Web address'],
            ] as const
          ).map(([k, label]) => {
            const on = kind === k;
            return (
              <Pressable
                key={k}
                onPress={() => setKind(k)}
                style={[styles.segment, on && styles.segmentOn]}
                accessibilityRole="radio"
                accessibilityState={{ selected: on }}
              >
                {on ? <Icon name="check" size={16} color={m3.onSecondaryContainer} /> : null}
                <Text style={[type.labelLarge, { color: on ? m3.onSecondaryContainer : m3.onSurface }]}>{label}</Text>
              </Pressable>
            );
          })}
        </View>
        {kind === 'stdio' ? (
          <>
            <Field
              label="Command"
              hint="The program that starts the server, often npx (Node.js) or uvx (Python)."
              value={s.command}
              onChangeText={(v) => set({ command: v })}
              placeholder="npx"
              style={{ fontFamily: mono }}
            />
            <Field
              label="Arguments, one per line"
              value={args}
              onChangeText={setArgs}
              placeholder={'-y\n@modelcontextprotocol/server-github'}
              multiline
              style={{ fontFamily: mono }}
            />
            <Field
              label="Environment variables, one KEY=value per line"
              hint="API keys and other settings the server's instructions ask for. Saved in Relay's settings on this PC."
              value={env}
              onChangeText={setEnv}
              placeholder="GITHUB_PERSONAL_ACCESS_TOKEN=ghp_..."
              multiline
              style={{ fontFamily: mono }}
            />
          </>
        ) : (
          <>
            <Field
              label="Address"
              hint="The server's Streamable HTTP address, usually ending in /mcp."
              value={s.url}
              onChangeText={(v) => set({ url: v })}
              placeholder="https://example.com/mcp"
              keyboardType="url"
              style={{ fontFamily: mono }}
            />
            <Field
              label="Headers, one Name: value per line"
              hint="For servers that need a sign-in token."
              value={headers}
              onChangeText={setHeaders}
              placeholder="Authorization: Bearer ..."
              multiline
              style={{ fontFamily: mono }}
            />
          </>
        )}
        <Field
          label="Note (optional)"
          value={s.description}
          onChangeText={(v) => set({ description: v })}
          placeholder="My GitHub repositories"
        />
        <Field
          label="Timeout in seconds"
          hint="How long one tool call may take. Leave empty for 120."
          value={timeout}
          onChangeText={(v) => setTimeoutText(v.replace(/[^0-9]/g, ''))}
          placeholder="120"
          keyboardType="number-pad"
        />

        <Button
          title="Check it connects"
          icon="connection"
          variant="outlined"
          onPress={check}
          loading={testing}
          disabled={!ready || testing}
        />
        {testing ? (
          <Text style={[type.bodySmall, styles.muted]}>
            The first start can take a minute while npx or uvx downloads the server.
          </Text>
        ) : null}
        {test?.ok ? (
          <View style={styles.okBox}>
            <Text style={type.bodyMedium}>
              Connected{test.server_info?.name ? ` to ${test.server_info.name}` : ''}. It has {test.tools.length}{' '}
              {test.tools.length === 1 ? 'tool' : 'tools'}
              {test.tools.length ? `: ${test.tools.map((t) => t.name).join(', ')}` : ''}.
            </Text>
          </View>
        ) : null}
        {test && !test.ok ? <Banner message={test.error ?? 'Could not connect'} /> : null}
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

/** Paste the `"mcpServers"` JSON most MCP servers' instructions give. */
function ImportSheet({ client, onClose, onDone }: { client: RelayClient; onClose: () => void; onDone: (servers: McpServerView[]) => void }) {
  const [text, setText] = useState('');
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const run = async () => {
    setBusy(true);
    setError(null);
    try {
      const r = await client.importMcpServers(text);
      onDone(r.servers);
      if (r.skipped.length) {
        alert('Some were already added', `Skipped ${r.skipped.join(', ')}: a server with that name exists.`);
      }
    } catch (e) {
      setError(describeError(e));
    } finally {
      setBusy(false);
    }
  };
  return (
    <Sheet visible onClose={onClose} title="Paste a server config">
      <ScrollView style={styles.sheetScroll} contentContainerStyle={styles.sheetBody} keyboardShouldPersistTaps="handled">
        <Text style={[type.bodyMedium, styles.muted]}>
          Most MCP servers show a config for Claude Desktop, Cursor or VS Code. Paste it here as it is.
        </Text>
        <Field
          label="Config (JSON)"
          value={text}
          onChangeText={setText}
          placeholder={'{\n  "mcpServers": {\n    "time": { "command": "uvx", "args": ["mcp-server-time"] }\n  }\n}'}
          multiline
          style={{ fontFamily: mono, minHeight: 160 }}
        />
        {error ? <Banner message={error} /> : null}
      </ScrollView>
      <View style={styles.sheetButtons}>
        <View style={styles.grow} />
        <Button title="Cancel" variant="text" onPress={onClose} />
        <Button title="Add" icon="check" onPress={run} loading={busy} disabled={!text.trim() || busy} />
      </View>
    </Sheet>
  );
}

/**
 * The MCP servers section of Settings > Tools: servers others publish
 * (GitHub, databases, Home Assistant...) whose tools the agent can use.
 */
export function McpSection({ client }: { client: RelayClient }) {
  const [servers, setServers] = useState<McpServerView[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [open, setOpen] = useState<string | null>(null);
  const [editor, setEditor] = useState<{ server: McpServer; editing: string | null; key: number } | null>(null);
  const [importing, setImporting] = useState(false);
  const alive = useRef(true);

  const load = useCallback(async () => {
    try {
      const r = await client.mcpServers();
      if (alive.current) {
        setServers(r.servers);
        setError(null);
      }
    } catch (e) {
      if (alive.current) setError(describeError(e));
    }
  }, [client]);

  useEffect(() => {
    alive.current = true;
    load();
    return () => {
      alive.current = false;
    };
  }, [load]);

  // Follow servers that are still connecting.
  const starting = servers?.some((s) => s.enabled && s.status.state === 'starting');
  useEffect(() => {
    if (!starting) return;
    const t = setInterval(load, 1500);
    return () => clearInterval(t);
  }, [starting, load]);

  const replace = (v: McpServerView, oldName = v.name) =>
    setServers((list) => (list ? list.map((s) => (s.name === oldName ? v : s)) : list));

  const toggle = async (s: McpServerView, on: boolean) => {
    setBusy(s.name);
    try {
      replace(await client.updateMcpServer(s.name, { ...toConfig(s), enabled: on }));
    } catch (e) {
      alert('Could not change the server', describeError(e));
    } finally {
      setBusy(null);
    }
  };

  const save = async (cfg: McpServer) => {
    if (editor?.editing) {
      const v = await client.updateMcpServer(editor.editing, cfg);
      replace(v, editor.editing);
      if (open === editor.editing) setOpen(v.name);
    } else {
      const v = await client.addMcpServer(cfg);
      setServers((list) => [...(list ?? []), v]);
    }
    setEditor(null);
  };

  const remove = () => {
    const name = editor?.editing;
    if (!name) return;
    alert(`Delete ${name}?`, 'Relay stops the server and the agent can no longer use its tools.', [
      { text: 'Cancel', style: 'cancel' },
      {
        text: 'Delete',
        style: 'destructive',
        onPress: async () => {
          try {
            setServers((await client.deleteMcpServer(name)).servers);
            setEditor(null);
            setOpen(null);
          } catch (e) {
            alert('Could not delete the server', describeError(e));
          }
        },
      },
    ]);
  };

  const current = servers?.find((s) => s.name === open) ?? null;

  return (
    <>
      <SectionTitle title="MCP servers" />
      <Group>
        {!servers && !error ? <ActivityIndicator color={m3.primary} style={styles.pad} /> : null}
        {error ? <Banner message={error} actionLabel="Retry" onAction={load} /> : null}
        {servers && !servers.length ? (
          <View style={styles.empty}>
            <Icon name="puzzle-outline" size={32} color={m3.outline} />
            <Text style={[type.bodyMedium, styles.muted, styles.center]}>
              MCP servers give the agent ready-made tools for other apps and services, like GitHub, a database or Home
              Assistant.
            </Text>
          </View>
        ) : null}
        {servers?.map((s, i) => (
          <ServerRow
            key={s.name}
            server={s}
            first={i === 0}
            busy={busy === s.name}
            onToggle={(v) => toggle(s, v)}
            onOpen={() => setOpen(s.name)}
          />
        ))}
        {servers ? (
          <View style={styles.addRow}>
            <Button
              title="Add a server"
              icon="plus"
              variant="tonal"
              onPress={() => setEditor({ server: EMPTY, editing: null, key: Date.now() })}
            />
            <Button title="Paste a config" icon="content-paste" variant="text" onPress={() => setImporting(true)} />
            <Button
              title="Browse the registry"
              icon="storefront-outline"
              variant="text"
              onPress={() => router.push({ pathname: '/browse', params: { tab: 'mcp' } })}
            />
          </View>
        ) : null}
      </Group>
      {servers?.length ? (
        <Text style={[type.bodySmall, styles.footnote]}>
          Servers run on this PC and their tools work in every chat. Tap one to see its tools.
        </Text>
      ) : null}

      {current && !editor ? (
        <ServerSheet
          server={current}
          onClose={() => setOpen(null)}
          onChange={async (cfg) => replace(await client.updateMcpServer(current.name, cfg))}
          onEdit={() => setEditor({ server: toConfig(current), editing: current.name, key: Date.now() })}
          onRestart={async () => {
            try {
              replace(await client.restartMcpServer(current.name));
            } catch (e) {
              alert('Could not restart', describeError(e));
            }
          }}
        />
      ) : null}
      {editor ? (
        <ServerEditor
          key={editor.key}
          client={client}
          initial={editor.server}
          editing={editor.editing}
          onClose={() => setEditor(null)}
          onSave={save}
          onDelete={remove}
        />
      ) : null}
      {importing ? (
        <ImportSheet
          client={client}
          onClose={() => setImporting(false)}
          onDone={(list) => {
            setServers(list);
            setImporting(false);
          }}
        />
      ) : null}
    </>
  );
}

const styles = StyleSheet.create({
  pad: { margin: space.lg },
  empty: { alignItems: 'center', gap: space.sm, padding: space.xl },
  muted: { color: m3.onSurfaceVariant },
  center: { textAlign: 'center' },
  grow: { flex: 1, minWidth: 0 },
  mono: { fontFamily: mono },
  footnote: { color: m3.onSurfaceVariant, marginTop: space.sm, paddingHorizontal: space.lg },
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
  stateLine: { flexDirection: 'row', alignItems: 'center', gap: space.xs, marginTop: 2 },
  dot: { width: 8, height: 8, borderRadius: 4 },
  addRow: { flexDirection: 'row', flexWrap: 'wrap', gap: space.sm, paddingHorizontal: space.lg, paddingVertical: space.md },
  code: { fontFamily: mono, fontSize: 13, color: m3.onSurfaceVariant },
  launch: { backgroundColor: m3.surfaceContainer, borderRadius: radius.md, padding: space.md },
  errorBox: { maxHeight: 160, backgroundColor: m3.errorContainer, borderRadius: radius.md, padding: space.md },
  okBox: { backgroundColor: m3.surfaceContainerHigh, borderRadius: radius.md, padding: space.md },
  trustRow: { flexDirection: 'row', alignItems: 'center', gap: space.md },
  toolBox: { borderWidth: 1, borderColor: m3.outlineVariant, borderRadius: radius.lg, padding: space.md, gap: space.xs },
  toolHead: { flexDirection: 'row', alignItems: 'center', gap: space.md },
  toolName: { fontFamily: mono, fontSize: 14, color: m3.onSurface },
  params: { gap: 4, marginTop: space.xs },
  paramLine: { flexDirection: 'row', gap: space.sm },
  paramName: { fontFamily: mono, fontSize: 12, color: m3.onSurface, minWidth: 112 },
  sheetScroll: { maxHeight: 520, flexGrow: 0 },
  sheetBody: { gap: space.md, paddingBottom: space.md },
  sheetButtons: { flexDirection: 'row', alignItems: 'center', gap: space.sm, paddingTop: space.md },
  segments: { flexDirection: 'row', gap: space.sm, flexWrap: 'wrap' },
  segment: {
    flexDirection: 'row',
    alignItems: 'center',
    gap: 6,
    paddingHorizontal: space.md,
    height: 36,
    borderRadius: radius.md,
    borderWidth: 1,
    borderColor: m3.outline,
  },
  segmentOn: { backgroundColor: m3.secondaryContainer, borderColor: m3.secondaryContainer },
});
