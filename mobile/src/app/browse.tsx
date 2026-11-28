import { router, useFocusEffect, useLocalSearchParams } from 'expo-router';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import {
  ActivityIndicator,
  KeyboardAvoidingView,
  Linking,
  Platform,
  Pressable,
  ScrollView,
  StyleSheet,
  Text,
  View,
} from 'react-native';

import { alert } from '../components/Dialogs';
import { Group, SectionTitle } from '../components/SettingsControls';
import { Sheet } from '../components/Sheet';
import { m3, mono, radius, space, type } from '../components/theme';
import { Banner, Button, Field, Icon, type IconName } from '../components/ui';
import { ApiError, describeError } from '../lib/api';
import { useClient } from '../lib/connection';
import { usePageStyle } from '../lib/layout';
import type {
  CatalogSkill,
  CatalogSkills,
  CatalogTool,
  McpEntry,
  McpOption,
  McpServer,
  McpServerView,
} from '../lib/types';

type Tab = 'skills' | 'tools' | 'mcp';

const TABS: { value: Tab; label: string; icon: IconName }[] = [
  { value: 'skills', label: 'Skills', icon: 'puzzle-outline' },
  { value: 'tools', label: 'Tools', icon: 'wrench-outline' },
  { value: 'mcp', label: 'MCP servers', icon: 'connection' },
];

function size(bytes: number) {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

function matches(q: string, ...texts: string[]) {
  const words = q.toLowerCase().split(/\s+/).filter(Boolean);
  const hay = texts.join(' ').toLowerCase();
  return words.every((w) => hay.includes(w));
}

function Tabs({ value, onChange }: { value: Tab; onChange: (t: Tab) => void }) {
  return (
    <View style={styles.tabs} accessibilityRole="tablist">
      {TABS.map((t) => {
        const on = t.value === value;
        return (
          <Pressable
            key={t.value}
            onPress={() => onChange(t.value)}
            style={({ pressed }) => [styles.tab, on && styles.tabOn, pressed && styles.pressed]}
            accessibilityRole="tab"
            accessibilityState={{ selected: on }}
          >
            <Icon name={t.icon} size={18} color={on ? m3.onSecondaryContainer : m3.onSurfaceVariant} />
            <Text style={[type.labelLarge, { color: on ? m3.onSecondaryContainer : m3.onSurface }]} numberOfLines={1}>
              {t.label}
            </Text>
          </Pressable>
        );
      })}
    </View>
  );
}

function Chip({
  label,
  selected,
  icon,
  onPress,
}: {
  label: string;
  selected?: boolean;
  icon?: IconName;
  onPress: () => void;
}) {
  return (
    <Pressable
      onPress={onPress}
      style={({ pressed }) => [styles.chip, selected && styles.chipOn, pressed && styles.pressed]}
      accessibilityRole="button"
      accessibilityState={{ selected: !!selected }}
    >
      {selected ? <Icon name="check" size={16} color={m3.onSecondaryContainer} /> : null}
      {!selected && icon ? <Icon name={icon} size={16} color={m3.primary} /> : null}
      <Text style={[type.labelLarge, { color: selected ? m3.onSecondaryContainer : m3.onSurface }]} numberOfLines={1}>
        {label}
      </Text>
    </Pressable>
  );
}

function Divider({ first }: { first: boolean }) {
  return first ? null : <View style={styles.divider} />;
}

function Link({ label, url }: { label: string; url: string }) {
  return (
    <Text style={[type.bodySmall, styles.link]} onPress={() => Linking.openURL(url)} accessibilityRole="link">
      {label}
    </Text>
  );
}

// ---------- skills ----------

function SkillRow({
  skill,
  first,
  busy,
  onInstall,
  onRemove,
}: {
  skill: CatalogSkill;
  first: boolean;
  busy: boolean;
  onInstall: () => void;
  onRemove: () => void;
}) {
  const inst = skill.installed;
  return (
    <View>
      <Divider first={first} />
      <View style={styles.row}>
        <Icon name="puzzle-outline" size={22} color={m3.onSurfaceVariant} />
        <View style={styles.grow}>
          <Text style={type.bodyLarge}>{skill.name}</Text>
          {skill.description ? (
            <Text style={[type.bodyMedium, styles.muted]} numberOfLines={3}>
              {skill.description}
            </Text>
          ) : null}
          <View style={styles.meta}>
            <Text style={[type.bodySmall, styles.outline]}>
              {skill.files} {skill.files === 1 ? 'file' : 'files'} · {size(skill.bytes)}
            </Text>
            <Link label="View on GitHub" url={skill.url} />
          </View>
          {skill.name_taken ? (
            <Text style={[type.bodySmall, styles.warn]}>You already have a skill called {skill.name}.</Text>
          ) : null}
        </View>
        <View style={styles.actions}>
          {busy ? (
            <ActivityIndicator color={m3.primary} />
          ) : inst ? (
            <>
              {inst.update_available ? (
                <Button title="Update" icon="update" variant="tonal" onPress={onInstall} />
              ) : (
                <View style={styles.installed}>
                  <Icon name="check-circle" size={18} color={m3.primary} />
                  <Text style={[type.labelLarge, { color: m3.primary }]}>Installed</Text>
                </View>
              )}
              <Button title="Remove" variant="text" onPress={onRemove} />
            </>
          ) : (
            <Button
              title="Install"
              icon="download"
              variant="tonal"
              onPress={onInstall}
              disabled={skill.name_taken}
              accessibilityLabel={`Install ${skill.name}`}
            />
          )}
        </View>
      </View>
    </View>
  );
}

function AddSourceSheet({
  visible,
  onClose,
  onAdd,
}: {
  visible: boolean;
  onClose: () => void;
  onAdd: (source: string) => Promise<void>;
}) {
  const [text, setText] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const submit = async () => {
    if (!text.trim()) return;
    setBusy(true);
    setError(null);
    try {
      await onAdd(text.trim());
      setText('');
    } catch (e) {
      setError(describeError(e));
    } finally {
      setBusy(false);
    }
  };
  return (
    <Sheet visible={visible} onClose={onClose} title="Add a skill collection">
      <View style={styles.sheetBody}>
        <Text style={[type.bodyMedium, styles.muted]}>
          Any public GitHub repository with skill folders in it (each one a folder with a SKILL.md).
        </Text>
        <Field
          label="GitHub repository"
          placeholder="owner/repo or a github.com link"
          value={text}
          onChangeText={setText}
          onSubmitEditing={submit}
          autoFocus
          icon="github"
          hint="A link to a folder or branch works too, e.g. github.com/owner/repo/tree/main/skills"
        />
        {error ? <Banner message={error} /> : null}
        <View style={styles.sheetButtons}>
          <View style={styles.grow} />
          <Button title="Cancel" variant="text" onPress={onClose} />
          <Button title="Add" onPress={submit} loading={busy} disabled={!text.trim()} />
        </View>
      </View>
    </Sheet>
  );
}

function SkillsTab({ query }: { query: string }) {
  const client = useClient();
  const [source, setSource] = useState<string | undefined>(undefined);
  const [data, setData] = useState<CatalogSkills | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [adding, setAdding] = useState(false);
  const seq = useRef(0);

  const load = useCallback(
    async (src?: string, refresh = false) => {
      if (!client) return;
      const n = ++seq.current;
      setLoading(true);
      try {
        const d = await client.catalogSkills(src, refresh);
        if (n !== seq.current) return;
        setData(d);
        setError(null);
      } catch (e) {
        if (n === seq.current) setError(describeError(e));
      } finally {
        if (n === seq.current) setLoading(false);
      }
    },
    [client],
  );

  useEffect(() => {
    load(source);
  }, [load, source]);

  if (!client) return null;
  const current = data?.source ?? source;
  const sources = data?.sources ?? [];
  const currentSource = sources.find((s) => s.source === current);
  const skills = (data?.skills ?? []).filter((k) => matches(query, k.name, k.description, k.path));

  const install = async (k: CatalogSkill) => {
    if (!data) return;
    setBusy(k.path);
    try {
      await client.installSkill(data.source, k.path);
      await load(data.source);
    } catch (e) {
      alert(`Could not install ${k.name}`, describeError(e));
    } finally {
      setBusy(null);
    }
  };

  const remove = (k: CatalogSkill) => {
    const folder = k.installed?.folder;
    if (!folder) return;
    alert(`Remove ${k.name}?`, 'The agent can no longer use it. You can install it again any time.', [
      { text: 'Cancel', style: 'cancel' },
      {
        text: 'Remove',
        style: 'destructive',
        onPress: async () => {
          setBusy(k.path);
          try {
            await client.uninstallSkill(folder);
            await load(data?.source);
          } catch (e) {
            alert('Could not remove the skill', describeError(e));
          } finally {
            setBusy(null);
          }
        },
      },
    ]);
  };

  const removeSource = () => {
    if (!current || currentSource?.builtin) return;
    alert(`Remove ${current}?`, 'Skills you installed from it stay installed.', [
      { text: 'Cancel', style: 'cancel' },
      {
        text: 'Remove',
        style: 'destructive',
        onPress: async () => {
          try {
            await client.removeSkillSource(current);
            setSource(undefined);
            if (source === undefined) load();
          } catch (e) {
            alert('Could not remove the collection', describeError(e));
          }
        },
      },
    ]);
  };

  const installedCount = data?.skills.filter((k) => k.installed).length ?? 0;

  return (
    <View>
      <ScrollView horizontal showsHorizontalScrollIndicator={false} contentContainerStyle={styles.chips}>
        {sources.map((s) => (
          <Chip key={s.source} label={s.source} selected={s.source === current} onPress={() => setSource(s.source)} />
        ))}
        <Chip label="Add collection" icon="plus" onPress={() => setAdding(true)} />
      </ScrollView>

      {error ? <Banner message={error} actionLabel="Retry" onAction={() => load(source)} /> : null}
      {data?.error ? <Banner message={data.error} actionLabel="Retry" onAction={() => load(source, true)} /> : null}

      <View style={styles.sourceHead}>
        <Text style={[type.bodySmall, styles.muted, styles.grow]}>
          {data && !data.error
            ? `${data.skills.length} skills${installedCount ? `, ${installedCount} installed` : ''}`
            : ' '}
        </Text>
        {current && !currentSource?.builtin && currentSource ? (
          <Button title="Remove collection" variant="text" onPress={removeSource} />
        ) : null}
        <Button title="Refresh" icon="refresh" variant="text" onPress={() => load(current, true)} disabled={loading} />
      </View>

      <Group>
        {loading && !data?.skills.length ? <ActivityIndicator color={m3.primary} style={styles.pad} /> : null}
        {data && !loading && !data.error && !skills.length ? (
          <View style={styles.empty}>
            <Text style={[type.bodyMedium, styles.muted, styles.center]}>No skills match “{query}”.</Text>
          </View>
        ) : null}
        {skills.map((k, i) => (
          <SkillRow
            key={k.path}
            skill={k}
            first={i === 0}
            busy={busy === k.path}
            onInstall={() => install(k)}
            onRemove={() => remove(k)}
          />
        ))}
      </Group>
      <Text style={[type.bodySmall, styles.footnote]}>
        Skills install into the skills folder next to config.toml. The agent sees each skill&apos;s name and description
        and reads the rest when a task needs it. A skill can ask the agent to run its scripts, so install ones you
        trust; your approval settings still apply.
      </Text>

      <AddSourceSheet
        visible={adding}
        onClose={() => setAdding(false)}
        onAdd={async (s) => {
          const r = await client.addSkillSource(s);
          setAdding(false);
          setSource(r.source);
        }}
      />
    </View>
  );
}

// ---------- tools ----------

function ToolsTab({ query }: { query: string }) {
  const client = useClient();
  const [data, setData] = useState<{ tools: CatalogTool[]; shell: string } | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);

  const load = useCallback(async () => {
    if (!client) return;
    try {
      setData(await client.catalogTools());
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

  const groups = useMemo(() => {
    const out = new Map<string, CatalogTool[]>();
    for (const t of data?.tools ?? []) {
      if (!matches(query, t.title, t.tool.name, t.tool.description, t.category)) continue;
      out.set(t.category, [...(out.get(t.category) ?? []), t]);
    }
    return [...out.entries()];
  }, [data, query]);

  if (!client) return null;

  const add = async (t: CatalogTool) => {
    setBusy(t.tool.name);
    try {
      await client.addTool(t.tool);
      await load();
    } catch (e) {
      alert(`Could not add ${t.title}`, describeError(e));
    } finally {
      setBusy(null);
    }
  };

  return (
    <View>
      {error ? <Banner message={error} actionLabel="Retry" onAction={load} /> : null}
      {!data && !error ? <ActivityIndicator color={m3.primary} style={styles.pad} /> : null}
      {data && !groups.length ? (
        <Text style={[type.bodyMedium, styles.muted, styles.center, styles.pad]}>No tools match “{query}”.</Text>
      ) : null}
      {groups.map(([category, tools]) => (
        <View key={category}>
          <SectionTitle title={category} />
          <Group>
            {tools.map((t, i) => (
              <View key={t.tool.name}>
                <Divider first={i === 0} />
                <View style={styles.row}>
                  <Icon name="wrench-outline" size={22} color={m3.onSurfaceVariant} />
                  <View style={styles.grow}>
                    <Text style={type.bodyLarge}>{t.title}</Text>
                    <Text style={styles.code}>{t.tool.name}</Text>
                    <Text style={[type.bodyMedium, styles.muted]} numberOfLines={3}>
                      {t.tool.description}
                    </Text>
                    <Text style={[styles.code, styles.commandLine]} numberOfLines={2} ellipsizeMode="tail">
                      {t.tool.command}
                    </Text>
                  </View>
                  <View style={styles.actions}>
                    {busy === t.tool.name ? (
                      <ActivityIndicator color={m3.primary} />
                    ) : t.installed ? (
                      <View style={styles.installed}>
                        <Icon name="check-circle" size={18} color={m3.primary} />
                        <Text style={[type.labelLarge, { color: m3.primary }]}>Added</Text>
                      </View>
                    ) : (
                      <Button
                        title="Add"
                        icon="plus"
                        variant="tonal"
                        onPress={() => add(t)}
                        accessibilityLabel={`Add ${t.title}`}
                      />
                    )}
                  </View>
                </View>
              </View>
            ))}
          </Group>
        </View>
      ))}
      {data ? (
        <Text style={[type.bodySmall, styles.footnote]}>
          Written for {/powershell|pwsh/i.test(data.shell) ? 'PowerShell' : data.shell}. Added tools show up under Your
          tools, where you can change or turn them off. They run like any command, so your approval settings apply.
        </Text>
      ) : null}
      {data ? (
        <View style={styles.addRow}>
          <Button title="Open Tools" icon="arrow-right" variant="text" onPress={() => router.push('/tools')} />
        </View>
      ) : null}
    </View>
  );
}

// ---------- MCP servers ----------

/** The server to save from a registry option and what the user typed (by input index). */
function buildServer(o: McpOption, values: Record<number, string>) {
  const args = [...o.args];
  const env: Record<string, string> = {};
  const headers: Record<string, string> = {};
  const drop = new Set<number>();
  const placeholders: Record<number, string> = {};
  o.inputs.forEach((inp, i) => {
    const v = (values[i] ?? '').trim();
    const key = inp.key || inp.name;
    const fill = (current: string | undefined) =>
      inp.template ? (current ?? inp.template).split(`{${inp.name}}`).join(v) : v;
    if (inp.target === 'arg' && inp.arg_index !== undefined) {
      const at = inp.arg_index;
      if (!v) {
        if (!inp.required) {
          drop.add(at);
          if (inp.drop_index !== undefined) drop.add(inp.drop_index);
        } else placeholders[at] = `<${inp.name}>`;
        return;
      }
      args[at] = inp.template ? args[at].split(`{${inp.name}}`).join(v) : v;
    } else if (inp.target === 'env') {
      if (v) env[key] = fill(env[key]);
    } else if (inp.target === 'header') {
      // "Bearer " with nothing after it means no token.
      if (v && !/^bearer$/i.test(v)) headers[key] = fill(headers[key]);
    }
  });
  const finalArgs = args.map((a, i) => placeholders[i] ?? a).filter((_, i) => !drop.has(i));
  return { args: finalArgs, env, headers };
}

function McpInstallSheet({
  entry,
  taken,
  onClose,
  onInstalled,
}: {
  entry: McpEntry;
  taken: string[];
  onClose: () => void;
  onInstalled: () => void;
}) {
  const client = useClient();
  const supported = entry.options.filter((o) => o.supported);
  const [index, setIndex] = useState(0);
  const option = supported[index];
  const [name, setName] = useState(() => {
    if (!taken.includes(entry.name)) return entry.name;
    for (let n = 2; ; n++) if (!taken.includes(`${entry.name}_${n}`)) return `${entry.name}_${n}`;
  });
  const [values, setValues] = useState<Record<string, Record<number, string>>>({});
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [status, setStatus] = useState<McpServerView['status'] | null>(null);
  const alive = useRef(true);
  useEffect(
    () => () => {
      alive.current = false;
    },
    [],
  );
  if (!client || !option) return null;

  const optKey = `${index}`;
  const vals: Record<number, string> = {};
  option.inputs.forEach((inp, i) => (vals[i] = values[optKey]?.[i] ?? inp.default));
  const built = buildServer(option, vals);
  const missing = option.inputs.filter((inp, i) => inp.required && !vals[i]?.trim());
  const nameError =
    !/^[a-z][a-z0-9_]{0,31}$/.test(name) || name.includes('__')
      ? 'A lowercase letter first, then a-z, 0-9 or single _ (at most 32)'
      : taken.includes(name)
        ? 'You already have a server with this name'
        : null;

  const install = async () => {
    const server: McpServer = {
      name,
      description: entry.title ? `${entry.title}: ${entry.description}` : entry.description,
      enabled: true,
      command: option.kind === 'remote' ? '' : option.command,
      args: option.kind === 'remote' ? [] : built.args,
      env: built.env,
      cwd: '',
      url: option.kind === 'remote' ? option.url : '',
      headers: built.headers,
      timeout_seconds: 0,
      disabled_tools: [],
      trusted: false,
    };
    setBusy(true);
    setError(null);
    try {
      const saved = await client.addMcpServer(server);
      setStatus(saved.status);
      onInstalled();
      // Wait for it to start: the first run may download the package.
      for (let i = 0; i < 90 && alive.current; i++) {
        const { servers } = await client.mcpServers();
        const mine = servers.find((x) => x.name === name);
        if (!mine) break;
        setStatus(mine.status);
        if (mine.status.state === 'ready' || mine.status.state === 'error') break;
        await new Promise((r) => setTimeout(r, 2000));
      }
    } catch (e) {
      setError(describeError(e));
    } finally {
      setBusy(false);
    }
  };

  const remove = async () => {
    try {
      await client.deleteMcpServer(name);
      onInstalled();
      onClose();
    } catch (e) {
      setError(describeError(e));
    }
  };

  const preview =
    option.kind === 'remote'
      ? option.url
      : [option.command, ...built.args.map((a) => (/\s/.test(a) ? `"${a}"` : a))].join(' ');
  const secrets = option.inputs
    .map((inp, i) => (inp.secret ? vals[i]?.trim() : ''))
    .filter((v): v is string => !!v && v.length > 3);
  const shown = secrets.reduce((t, sec) => t.split(sec).join('••••'), preview);

  if (status) {
    const tools = status.tools ?? [];
    return (
      <Sheet visible onClose={onClose} title={`Install ${entry.title || entry.name}`}>
        <View style={styles.sheetBody}>
          {status.state === 'ready' ? (
            <View style={styles.statusLine}>
              <Icon name="check-circle" size={22} color={m3.primary} />
              <Text style={[type.bodyLarge, styles.grow]}>
                Ready. The agent can use its {tools.length} {tools.length === 1 ? 'tool' : 'tools'}, named {name}__ and
                the tool name.
              </Text>
            </View>
          ) : status.state === 'error' ? (
            <Banner message={`It was saved but didn't start: ${status.error ?? 'unknown error'}`} />
          ) : (
            <View style={styles.statusLine}>
              <ActivityIndicator color={m3.primary} />
              <Text style={[type.bodyLarge, styles.grow]}>
                Starting… The first start can take a minute while the package downloads.
              </Text>
            </View>
          )}
          {tools.length ? (
            <Text style={[type.bodySmall, styles.muted]} numberOfLines={4}>
              {tools.map((t) => t.name).join(', ')}
            </Text>
          ) : null}
          {error ? <Banner message={error} /> : null}
        </View>
        <View style={styles.sheetButtons}>
          {status.state === 'error' ? <Button title="Remove it" variant="text" onPress={remove} /> : null}
          <View style={styles.grow} />
          <Button
            title="Open Tools"
            variant="text"
            onPress={() => {
              onClose();
              router.push('/tools');
            }}
          />
          <Button title="Done" onPress={onClose} />
        </View>
      </Sheet>
    );
  }

  return (
    <Sheet visible onClose={onClose} title={`Install ${entry.title || entry.name}`}>
      <ScrollView
        style={styles.sheetScroll}
        contentContainerStyle={styles.sheetBody}
        keyboardShouldPersistTaps="handled"
      >
        {supported.length > 1 ? (
          <View style={styles.optionList}>
            <Text style={[type.labelMedium, styles.muted]}>How to run it</Text>
            <View style={styles.chipsWrap}>
              {supported.map((o, i) => (
                <Chip key={`${o.kind}${i}`} label={o.label} selected={i === index} onPress={() => setIndex(i)} />
              ))}
            </View>
          </View>
        ) : null}
        <View style={styles.previewBox}>
          <Text style={[type.labelMedium, styles.muted]}>
            {option.kind === 'remote' ? 'Connects to' : 'Runs on this PC'}
          </Text>
          <Text style={styles.previewText}>{shown}</Text>
        </View>
        <Field
          label="Name"
          value={name}
          onChangeText={(t) => setName(t.toLowerCase().replace(/[^a-z0-9_]/g, '_'))}
          hint={nameError ?? 'The agent sees its tools as name__tool.'}
        />
        {option.inputs.map((inp, i) => (
          <Field
            key={`${optKey}:${i}`}
            label={`${inp.key && inp.key !== inp.name ? `${inp.key}: ` : ''}${inp.name}${inp.required ? '' : ' (optional)'}`}
            hint={inp.description || (inp.target === 'header' ? 'Sent with each request' : undefined)}
            value={vals[i] ?? ''}
            onChangeText={(t) => setValues((v) => ({ ...v, [optKey]: { ...v[optKey], [i]: t } }))}
            secureTextEntry={inp.secret}
          />
        ))}
        {option.command === 'npx' ? (
          <Text style={[type.bodySmall, styles.muted]}>Needs Node.js on this PC.</Text>
        ) : null}
        {option.command === 'uvx' ? (
          <Text style={[type.bodySmall, styles.muted]}>Needs uv (docs.astral.sh/uv) on this PC.</Text>
        ) : null}
        {option.command === 'docker' ? (
          <Text style={[type.bodySmall, styles.muted]}>Needs Docker running on this PC.</Text>
        ) : null}
        {error ? <Banner message={error} /> : null}
      </ScrollView>
      <View style={styles.sheetButtons}>
        <View style={styles.grow} />
        <Button title="Cancel" variant="text" onPress={onClose} />
        <Button
          title="Install"
          onPress={install}
          loading={busy}
          disabled={!!nameError || missing.length > 0}
          accessibilityLabel={`Install ${entry.name} now`}
        />
      </View>
    </Sheet>
  );
}

/** The saved server this registry entry was installed as, if any. */
function installedAs(e: McpEntry, servers: McpServerView[]) {
  return servers.find(
    (sv) =>
      e.options.some(
        (o) =>
          (o.kind === 'remote' && o.url && sv.url === o.url) ||
          (o.package &&
            sv.args.some(
              (a) =>
                a === o.package || [`${o.package}@`, `${o.package}==`, `${o.package}:`].some((p) => a.startsWith(p)),
            )),
      ) || sv.description.startsWith(`${e.title || e.name}: `),
  );
}

const PAGE = 30;

const MCP_SUGGESTIONS = ['GitHub', 'Playwright', 'Home Assistant', 'Notion', 'Postgres', 'Slack', 'Filesystem'];

function McpTab({ query, setQuery }: { query: string; setQuery: (q: string) => void }) {
  const client = useClient();
  const [results, setResults] = useState<McpEntry[] | null>(null);
  const [cursor, setCursor] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [mine, setMine] = useState<McpServerView[] | null | undefined>(undefined);
  const [installing, setInstalling] = useState<McpEntry | null>(null);
  const [shown, setShown] = useState(PAGE);
  const seq = useRef(0);

  const loadMine = useCallback(async () => {
    if (!client) return;
    try {
      const { servers } = await client.mcpServers();
      setMine(servers);
    } catch (e) {
      // An older Relay without MCP support.
      setMine(e instanceof ApiError && e.status === 404 ? null : undefined);
    }
  }, [client]);

  useFocusEffect(
    useCallback(() => {
      loadMine();
    }, [loadMine]),
  );

  const search = useCallback(
    async (q: string, after?: string) => {
      if (!client) return;
      const n = ++seq.current;
      setLoading(true);
      try {
        const r = await client.catalogMcp(q, after);
        if (n !== seq.current) return;
        setResults((prev) => (after && prev ? [...prev, ...r.servers] : r.servers));
        if (!after) setShown(PAGE);
        setCursor(r.next_cursor);
        setError(null);
      } catch (e) {
        if (n === seq.current) setError(describeError(e));
      } finally {
        if (n === seq.current) setLoading(false);
      }
    },
    [client],
  );

  useEffect(() => {
    if (!query.trim()) {
      seq.current++;
      setResults(null);
      setCursor(null);
      setLoading(false);
      return;
    }
    const t = setTimeout(() => search(query.trim()), 400);
    return () => clearTimeout(t);
  }, [query, search]);

  if (!client) return null;
  const canInstall = Array.isArray(mine);

  return (
    <View>
      {mine === null ? (
        <Banner
          kind="info"
          message="This Relay can't run MCP servers yet. You can look around now, and install once Relay is updated."
        />
      ) : null}
      {error ? <Banner message={error} actionLabel="Retry" onAction={() => search(query)} /> : null}
      {!query.trim() ? (
        <View style={styles.suggest}>
          <Text style={[type.bodyMedium, styles.muted]}>
            Search thousands of ready-made servers that give the agent new tools, like your GitHub, a database or your
            smart home. Try:
          </Text>
          <View style={styles.chipsWrap}>
            {MCP_SUGGESTIONS.map((q) => (
              <Chip key={q} label={q} icon="magnify" onPress={() => setQuery(q)} />
            ))}
          </View>
        </View>
      ) : null}
      {query.trim() ? (
        <Group>
          {loading && !results ? <ActivityIndicator color={m3.primary} style={styles.pad} /> : null}
          {results && !results.length && !loading ? (
            <View style={styles.empty}>
              <Text style={[type.bodyMedium, styles.muted, styles.center]}>
                {query ? `No servers match “${query}”.` : 'The registry is empty.'}
              </Text>
            </View>
          ) : null}
          {(results ?? []).slice(0, shown).map((e, i) => {
            const ok = e.options.find((o) => o.supported);
            const have = canInstall ? installedAs(e, mine) : undefined;
            return (
              <View key={e.id}>
                <Divider first={i === 0} />
                <View style={styles.row}>
                  <Icon name="connection" size={22} color={m3.onSurfaceVariant} />
                  <View style={styles.grow}>
                    <Text style={type.bodyLarge}>{e.title || e.name}</Text>
                    <Text style={styles.code} numberOfLines={1}>
                      {e.id}
                      {e.version ? ` · ${e.version}` : ''}
                    </Text>
                    {e.description ? (
                      <Text style={[type.bodyMedium, styles.muted]} numberOfLines={3}>
                        {e.description}
                      </Text>
                    ) : null}
                    <View style={styles.meta}>
                      {e.options.length ? (
                        <Text style={[type.bodySmall, styles.outline]}>
                          {[...new Set(e.options.filter((o) => o.supported).map((o) => o.label))].join(' · ') ||
                            'No way to run it here'}
                        </Text>
                      ) : null}
                      {e.repository ? <Link label="Source" url={e.repository} /> : null}
                      {e.website ? <Link label="Website" url={e.website} /> : null}
                    </View>
                    {!ok && e.options[0]?.reason ? (
                      <Text style={[type.bodySmall, styles.warn]}>Can&apos;t install: {e.options[0].reason}.</Text>
                    ) : null}
                  </View>
                  <View style={styles.actions}>
                    {have ? (
                      <View style={styles.installed}>
                        <Icon name="check-circle" size={18} color={m3.primary} />
                        <Text style={[type.labelLarge, { color: m3.primary }]}>Installed</Text>
                      </View>
                    ) : ok ? (
                      <Button
                        title="Install"
                        icon="download"
                        variant="tonal"
                        onPress={() => setInstalling(e)}
                        disabled={!canInstall}
                        accessibilityLabel={`Install ${e.name}`}
                      />
                    ) : null}
                  </View>
                </View>
              </View>
            );
          })}
        </Group>
      ) : null}
      {results && query.trim() && (shown < results.length || cursor) ? (
        <View style={styles.addRow}>
          <Button
            title="Show more"
            variant="text"
            loading={loading}
            onPress={() => {
              if (shown < results.length) setShown((n) => n + PAGE);
              else if (cursor) {
                setShown((n) => n + PAGE);
                search(query, cursor);
              }
            }}
          />
        </View>
      ) : null}
      <Text style={[type.bodySmall, styles.footnote]}>
        From the official MCP registry (registry.modelcontextprotocol.io). Anyone can publish there, and a server runs
        with your permissions on this PC, so install ones you trust. Relay asks before the agent uses a tool that
        changes things.
      </Text>
      {installing && canInstall ? (
        <McpInstallSheet
          entry={installing}
          taken={mine.map((m) => m.name)}
          onClose={() => setInstalling(null)}
          onInstalled={loadMine}
        />
      ) : null}
    </View>
  );
}

// ---------- screen ----------

export default function BrowseScreen() {
  const params = useLocalSearchParams<{ tab?: string }>();
  const page = usePageStyle();
  const [tab, setTab] = useState<Tab>(
    params.tab === 'tools' || params.tab === 'mcp' || params.tab === 'skills' ? params.tab : 'skills',
  );
  const [queries, setQueries] = useState<Record<Tab, string>>({ skills: '', tools: '', mcp: '' });
  const query = queries[tab];

  return (
    <KeyboardAvoidingView style={styles.flex} behavior={Platform.OS === 'ios' ? 'padding' : undefined}>
      <ScrollView contentContainerStyle={[styles.content, page]} keyboardShouldPersistTaps="handled">
        <Tabs value={tab} onChange={setTab} />
        <Field
          label=""
          icon="magnify"
          placeholder={
            tab === 'skills' ? 'Filter skills' : tab === 'tools' ? 'Filter tools' : 'Search the MCP registry'
          }
          value={query}
          onChangeText={(t) => setQueries((q) => ({ ...q, [tab]: t }))}
          returnKeyType="search"
        />
        {tab === 'skills' ? <SkillsTab query={query} /> : null}
        {tab === 'tools' ? <ToolsTab query={query} /> : null}
        {tab === 'mcp' ? <McpTab query={query} setQuery={(t) => setQueries((q) => ({ ...q, mcp: t }))} /> : null}
      </ScrollView>
    </KeyboardAvoidingView>
  );
}

const styles = StyleSheet.create({
  flex: { flex: 1, backgroundColor: m3.surface },
  content: { padding: space.lg, paddingBottom: 64, gap: space.sm },
  tabs: {
    flexDirection: 'row',
    borderWidth: 1,
    borderColor: m3.outline,
    borderRadius: radius.pill,
    overflow: 'hidden',
    marginBottom: space.sm,
  },
  tab: {
    flex: 1,
    flexDirection: 'row',
    alignItems: 'center',
    justifyContent: 'center',
    gap: space.xs,
    minHeight: 40,
    paddingHorizontal: space.sm,
  },
  tabOn: { backgroundColor: m3.secondaryContainer },
  chips: { gap: space.sm, paddingVertical: space.xs },
  suggest: { gap: space.md, paddingHorizontal: space.xs, paddingVertical: space.sm },
  chipsWrap: { flexDirection: 'row', flexWrap: 'wrap', gap: space.sm },
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
  sourceHead: { flexDirection: 'row', alignItems: 'center', gap: space.xs, minHeight: 40 },
  footnote: { color: m3.onSurfaceVariant, marginTop: space.sm, paddingHorizontal: space.lg },
  pad: { margin: space.lg },
  empty: { alignItems: 'center', gap: space.sm, padding: space.xl },
  muted: { color: m3.onSurfaceVariant },
  outline: { color: m3.outline },
  warn: { color: m3.warning, marginTop: 2 },
  center: { textAlign: 'center' },
  grow: { flex: 1, minWidth: 0 },
  divider: { height: StyleSheet.hairlineWidth, backgroundColor: m3.outlineVariant, marginLeft: 56 },
  row: {
    flexDirection: 'row',
    alignItems: 'flex-start',
    gap: space.lg,
    minHeight: 64,
    paddingHorizontal: space.lg,
    paddingVertical: space.md,
  },
  pressed: { backgroundColor: m3.pressed },
  meta: { flexDirection: 'row', flexWrap: 'wrap', alignItems: 'center', columnGap: space.md, marginTop: 2 },
  link: { color: m3.primary },
  actions: { alignItems: 'flex-end', gap: 2, minWidth: 96 },
  installed: { flexDirection: 'row', alignItems: 'center', gap: space.xs, minHeight: 40 },
  code: { fontFamily: mono, fontSize: 12, color: m3.onSurfaceVariant },
  commandLine: { marginTop: 4, color: m3.outline },
  addRow: { flexDirection: 'row', paddingHorizontal: space.sm, paddingVertical: space.sm },
  sheetScroll: { maxHeight: 520, flexGrow: 0 },
  sheetBody: { gap: space.md, paddingBottom: space.md },
  sheetButtons: { flexDirection: 'row', alignItems: 'center', gap: space.sm, paddingTop: space.md },
  optionList: { gap: space.sm },
  statusLine: { flexDirection: 'row', alignItems: 'center', gap: space.md },
  previewBox: {
    backgroundColor: m3.surfaceContainerHighest,
    borderRadius: radius.md,
    padding: space.md,
    gap: space.xs,
  },
  previewText: { fontFamily: mono, fontSize: 13, color: m3.onSurface },
});
