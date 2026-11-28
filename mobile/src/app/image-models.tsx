import { useFocusEffect } from 'expo-router';
import { useCallback, useEffect, useMemo, useState } from 'react';
import { ActivityIndicator, Linking, Pressable, ScrollView, StyleSheet, Text, TextInput, View } from 'react-native';
import { useSafeAreaInsets } from 'react-native-safe-area-context';

import { alert } from '../components/Dialogs';
import { ImageDownloads } from '../components/ImageDownloads';
import { Group, NavRow, SectionTitle, ToggleRow } from '../components/SettingsControls';
import { Sheet } from '../components/Sheet';
import { m3, mono, radius, space, type } from '../components/theme';
import { Banner, Button, Field, Icon, IconButton } from '../components/ui';
import { describeError } from '../lib/api';
import { formatBytes } from '../lib/format';
import { ModelUpdatesCard, useModelUpdates } from '../components/ModelUpdates';
import { useClient } from '../lib/connection';
import {
  imagesApi,
  setupComplete,
  type ImageCatalog,
  type ImageModel,
  type ImageSettings,
  type ImagesStatus,
  type Recipe,
} from '../lib/images';
import { usePageStyle } from '../lib/layout';

/**
 * Image models: the image engine (stable-diffusion.cpp), the folders it
 * reads models from, and saved setups (a model type plus the file for each
 * of its parts).
 */
export default function ImageModelsScreen() {
  const client = useClient();
  const api = useMemo(() => (client ? imagesApi(client) : null), [client]);
  const insets = useSafeAreaInsets();
  const page = usePageStyle();
  const [status, setStatus] = useState<ImagesStatus | null>(null);
  const [settings, setSettings] = useState<ImageSettings | null>(null);
  const [models, setModels] = useState<ImageModel[]>([]);
  const [catalog, setCatalog] = useState<ImageCatalog | null>(null);
  const [catalogError, setCatalogError] = useState<string | null>(null);
  const [loadingCatalog, setLoadingCatalog] = useState(false);
  const [setup, setSetup] = useState<{ model?: ImageModel; recipe?: string } | null>(null);
  const [addingFolder, setAddingFolder] = useState(false);
  const [adding, setAdding] = useState<string | null>(null);
  const [removing, setRemoving] = useState<string | null>(null);
  const updates = useModelUpdates(client);

  const loadCatalog = useCallback(
    async (refresh = false) => {
      if (!api) return;
      setLoadingCatalog(true);
      try {
        setCatalog(await api.catalog(refresh));
        setCatalogError(null);
      } catch (e) {
        setCatalogError(describeError(e));
      } finally {
        setLoadingCatalog(false);
      }
    },
    [api],
  );

  const load = useCallback(async () => {
    if (!api) return;
    api.status().then(setStatus).catch(() => undefined);
    api.settings().then(setSettings).catch(() => undefined);
    api.models().then(setModels).catch(() => undefined);
  }, [api]);

  useFocusEffect(
    useCallback(() => {
      load();
      loadCatalog();
      const t = setInterval(() => api?.status().then(setStatus).catch(() => undefined), 2000);
      return () => clearInterval(t);
    }, [load, loadCatalog, api]),
  );

  if (!api) return null;

  const patch = async (p: Partial<Omit<ImageSettings, 'own_folder'>>) => {
    try {
      setSettings(await api.patchSettings(p));
      if (p.model_dirs) setTimeout(() => loadCatalog(true), 800);
    } catch (e) {
      alert("Couldn't save", describeError(e));
    }
  };

  const remove = async (id: string, files: boolean) => {
    setRemoving(id);
    try {
      const r = await api.deleteModel(id, files);
      await load();
      if (files) {
        loadCatalog(true);
        const n = r.kept_files.length;
        const kept = n
          ? ` ${n === 1 ? 'One file was' : `${n} files were`} kept because another image model uses ${n === 1 ? 'it' : 'them'}.`
          : '';
        alert('Model deleted', `Freed ${formatBytes(r.freed_bytes)}.${kept}`);
      }
    } catch (e) {
      alert("Couldn't remove", describeError(e));
    } finally {
      setRemoving(null);
    }
  };

  const recipeName = (id: string) => catalog?.recipes.find((r) => r.id === id)?.name ?? id;
  const engine = status?.engine;
  const engineText = !engine
    ? 'Checking…'
    : engine.state === 'ready'
      ? engine.gpu === 'images'
        ? 'Running, image model on the GPU'
        : 'Running, waiting for a picture'
      : engine.state === 'idle'
        ? 'Stopped; starts when you make a picture'
        : engine.state === 'setup'
          ? (engine.message ?? 'Downloading…')
          : engine.state === 'starting'
            ? 'Starting…'
            : (engine.message ?? 'Problem');

  return (
    <ScrollView contentContainerStyle={[styles.content, page, { paddingBottom: insets.bottom + space.xxl }]}>
      <SectionTitle title="Image engine" />
      <Group>
        <View style={styles.engineRow}>
          <View style={[styles.engineIcon, engine?.state === 'error' && { backgroundColor: m3.errorContainer }]}>
            <Icon
              name={engine?.state === 'error' ? 'alert-circle-outline' : 'palette-outline'}
              color={engine?.state === 'error' ? m3.onErrorContainer : m3.primary}
            />
          </View>
          <View style={styles.grow}>
            <Text style={type.bodyLarge}>stable-diffusion.cpp</Text>
            <Text style={[type.bodyMedium, engine?.state === 'error' && { color: m3.error }]}>{engineText}</Text>
            {engine?.progress != null ? (
              <View style={styles.track}>
                <View style={[styles.fill, { width: `${Math.round(engine.progress * 100)}%` }]} />
              </View>
            ) : null}
          </View>
          {engine?.state === 'ready' && !engine.external ? (
            <Button title="Unload" variant="tonal" onPress={() => api.unload().then(load)} />
          ) : null}
        </View>
        <ToggleRow
          icon="memory"
          label="Use less VRAM"
          hint="Keeps model weights in RAM and moves them to the GPU as needed. Slower, but bigger models fit."
          value={!!settings?.offload_to_cpu}
          onChange={(v) => patch({ offload_to_cpu: v })}
        />
        <ToggleRow
          icon="timer-sand"
          label="Unload after 10 minutes unused"
          hint="Off keeps the image model loaded until a chat needs the GPU."
          value={(settings?.idle_unload_minutes ?? 10) > 0}
          onChange={(v) => patch({ idle_unload_minutes: v ? 10 : 0 })}
        />
      </Group>
      <Text style={[type.bodySmall, styles.note]}>
        The chat model and the image model take turns on the GPU. Making a picture unloads the chat model (after any
        chat that&apos;s working finishes), and the next chat message loads it again.
      </Text>

      <SectionTitle title="Your image models" />
      {client ? (
        <ModelUpdatesCard client={client} kind="image" updates={updates} onUpdated={() => loadCatalog(true)} />
      ) : null}
      {models.length === 0 ? (
        <Text style={[type.bodyMedium, styles.note]}>
          None yet. Download one below: it&apos;s set up for you once its files are in.
        </Text>
      ) : (
        <Group>
          {models.map((m) => (
            <View key={m.id} style={styles.modelRow}>
              <Pressable
                onPress={() => setSetup({ model: m })}
                style={({ pressed }) => [styles.modelMain, pressed && { backgroundColor: m3.pressed }]}
                accessibilityRole="button"
                accessibilityLabel={`${m.name}. Edit`}
              >
                <Icon name="image-filter-hdr" color={m3.onSurfaceVariant} />
                <View style={styles.grow}>
                  <Text style={type.bodyLarge} numberOfLines={1}>
                    {m.name}
                  </Text>
                  <Text style={type.bodyMedium} numberOfLines={1}>
                    {recipeName(m.recipe)} · {Object.keys(m.files).length} files
                  </Text>
                </View>
              </Pressable>
              {removing === m.id ? (
                <ActivityIndicator color={m3.primary} style={{ marginHorizontal: space.md }} />
              ) : (
                <IconButton
                  icon="trash-can-outline"
                  label={`Remove ${m.name}`}
                  disabled={removing !== null}
                  onPress={() =>
                    alert(
                      `Remove ${m.name}?`,
                      'Keep the files to set it up again later, or delete them from your PC to free the disk space. Files another image model uses are kept.',
                      [
                        { text: 'Cancel', style: 'cancel' },
                        { text: 'Keep files', onPress: () => remove(m.id, false) },
                        { text: 'Delete files', style: 'destructive', onPress: () => remove(m.id, true) },
                      ],
                    )
                  }
                />
              )}
            </View>
          ))}
        </Group>
      )}

      <ImageDownloads
        client={client!}
        onChanged={() => {
          load();
          loadCatalog(true);
        }}
      />

      {catalog?.suggestions.length ? (
        <>
          <SectionTitle title="Found in your folders" />
          <Group>
            {catalog.suggestions.map((s) => (
              <View key={`${s.recipe}:${s.files.diffusion_model ?? s.files.model}`} style={styles.modelRow}>
                <View style={[styles.modelMain, styles.grow]}>
                  <Icon name="folder-search-outline" color={m3.onSurfaceVariant} />
                  <View style={styles.grow}>
                    <Text style={type.bodyLarge} numberOfLines={1}>
                      {s.name}
                    </Text>
                    <Text style={type.bodyMedium} numberOfLines={1}>
                      {recipeName(s.recipe)} · every part found
                    </Text>
                  </View>
                </View>
                <Button
                  title="Add"
                  variant="tonal"
                  loading={adding === s.name}
                  style={{ marginRight: space.sm }}
                  onPress={async () => {
                    setAdding(s.name);
                    try {
                      await api.createModel(recipeName(s.recipe), s.recipe, s.files);
                      await load();
                      await loadCatalog();
                    } catch (e) {
                      alert("Couldn't add it", describeError(e));
                    } finally {
                      setAdding(null);
                    }
                  }}
                />
              </View>
            ))}
          </Group>
        </>
      ) : null}

      <View style={styles.buttons}>
        <Button
          title="Set up a model"
          icon="plus"
          onPress={() => setSetup({})}
          disabled={!catalog}
        />
        <Button
          title="Rescan folders"
          icon="refresh"
          variant="tonal"
          loading={loadingCatalog}
          onPress={() => loadCatalog(true)}
        />
      </View>
      {catalogError ? <Banner message={catalogError} actionLabel="Retry" onAction={() => loadCatalog(true)} /> : null}
      {!catalog && loadingCatalog && !catalogError ? (
        <View style={styles.loadingRow}>
          <ActivityIndicator color={m3.primary} />
          <Text style={[type.bodyMedium, styles.muted]}>Starting the image engine to read your folders…</Text>
        </View>
      ) : null}
      {catalog?.errors.length ? <Banner kind="warning" message={catalog.errors.slice(0, 3).join('\n')} /> : null}

      <SectionTitle title="Model folders" />
      <Group>
        {settings ? (
          <FolderRow path={settings.own_folder} note="Relay's own: put files in checkpoints, diffusion_models, text_encoders, vae and loras" />
        ) : null}
        {settings?.model_dirs.map((d) => (
          <FolderRow
            key={d}
            path={d}
            onRemove={() => patch({ model_dirs: settings.model_dirs.filter((x) => x !== d) })}
          />
        ))}
        <NavRow icon="folder-plus-outline" label="Add a models folder" value="For example your ComfyUI or SD WebUI models folder" onPress={() => setAddingFolder(true)} />
      </Group>
      <Text style={[type.bodySmall, styles.note]}>
        Most models come in parts: the main model, a text encoder and a VAE. Download a model above to get every part
        at once, or put files you already have in these folders and tap Rescan folders.
      </Text>

      <SetupSheet
        state={setup}
        catalog={catalog}
        onClose={() => setSetup(null)}
        onSave={async (name, recipe, files) => {
          if (setup?.model) await api.updateModel(setup.model.id, { name, files });
          else await api.createModel(name, recipe, files);
          setSetup(null);
          await load();
          loadCatalog();
        }}
      />
      <AddFolderSheet
        visible={addingFolder}
        onClose={() => setAddingFolder(false)}
        onAdd={async (path) => {
          await api.patchSettings({ model_dirs: [...(settings?.model_dirs ?? []), path] }).then(setSettings);
          setAddingFolder(false);
          setTimeout(() => loadCatalog(true), 800);
        }}
      />
    </ScrollView>
  );
}

function FolderRow({ path, note, onRemove }: { path: string; note?: string; onRemove?: () => void }) {
  return (
    <View style={styles.modelRow}>
      <View style={[styles.modelMain, styles.grow]}>
        <Icon name="folder-outline" color={m3.onSurfaceVariant} />
        <View style={styles.grow}>
          <Text style={[type.bodyMedium, { fontFamily: mono, color: m3.onSurface }]} numberOfLines={1} ellipsizeMode="middle">
            {path}
          </Text>
          {note ? <Text style={type.bodySmall}>{note}</Text> : null}
        </View>
      </View>
      {onRemove ? <IconButton icon="close" label={`Stop using ${path}`} onPress={onRemove} /> : null}
    </View>
  );
}

function AddFolderSheet({
  visible,
  onClose,
  onAdd,
}: {
  visible: boolean;
  onClose: () => void;
  onAdd: (path: string) => Promise<void>;
}) {
  const [path, setPath] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    if (visible) {
      setPath('');
      setError(null);
    }
  }, [visible]);
  return (
    <Sheet visible={visible} onClose={onClose} title="Add a models folder">
      <Field
        label="Folder on the PC"
        value={path}
        onChangeText={setPath}
        placeholder="C:\ComfyUI\models"
        hint="A folder with checkpoints, diffusion_models, text_encoders and vae inside, like ComfyUI's models folder"
        autoFocus
      />
      {error ? <Banner message={error} /> : null}
      <View style={styles.sheetActions}>
        <Button title="Cancel" variant="text" onPress={onClose} />
        <Button
          title="Add"
          loading={busy}
          disabled={!path.trim()}
          onPress={async () => {
            setBusy(true);
            try {
              await onAdd(path.trim());
            } catch (e) {
              setError(describeError(e));
            } finally {
              setBusy(false);
            }
          }}
        />
      </View>
    </Sheet>
  );
}

/** Pick a model type, then a file for each of its parts. */
function SetupSheet({
  state,
  catalog,
  onClose,
  onSave,
}: {
  state: { model?: ImageModel; recipe?: string } | null;
  catalog: ImageCatalog | null;
  onClose: () => void;
  onSave: (name: string, recipe: string, files: Record<string, string>) => Promise<void>;
}) {
  const [recipeId, setRecipeId] = useState<string | null>(null);
  const [files, setFiles] = useState<Record<string, string>>({});
  const [name, setName] = useState('');
  const [query, setQuery] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [slotOpen, setSlotOpen] = useState<string | null>(null);

  useEffect(() => {
    if (!state) return;
    setRecipeId(state.model?.recipe ?? state.recipe ?? null);
    setFiles(state.model?.files ?? {});
    setName(state.model?.name ?? '');
    setQuery('');
    setError(null);
    setSlotOpen(null);
  }, [state]);

  const recipe = catalog?.recipes.find((r) => r.id === recipeId) ?? null;
  const pickRecipe = (r: Recipe) => {
    setRecipeId(r.id);
    setName((n) => n || r.name);
    // Fill each part with the only (or best-named) file.
    const next: Record<string, string> = {};
    for (const slot of r.slots) {
      const cands = catalog?.files.filter((f) => slot.categories.includes(f.category)) ?? [];
      const words = `${r.id} ${slot.label}`.toLowerCase().split(/[^a-z0-9]+/).filter((w) => w.length > 1);
      const best = cands
        .map((f) => ({ f, s: words.filter((w) => f.name.toLowerCase().includes(w)).length }))
        .sort((a, b) => b.s - a.s)[0];
      if (best && (best.s > 0 || (slot.required && cands.length === 1))) next[slot.id] = best.f.id;
    }
    if (next.vae) delete next.tae;
    setFiles(next);
  };

  const recipes = (catalog?.recipes ?? []).filter(
    (r) => !query.trim() || `${r.name} ${r.id}`.toLowerCase().includes(query.trim().toLowerCase()),
  );

  return (
    <Sheet visible={!!state} onClose={onClose} title={state?.model ? 'Edit image model' : 'Set up an image model'}>
      <ScrollView style={{ maxHeight: 520 }} keyboardShouldPersistTaps="handled">
        {!recipe ? (
          <>
            <Text style={[type.bodyMedium, styles.muted, { marginBottom: space.sm }]}>
              Which kind of model is it? The file&apos;s download page says (for example Z-Image Turbo, FLUX.1 or SDXL).
            </Text>
            <View style={styles.search}>
              <Icon name="magnify" size={20} />
              <TextInput
                value={query}
                onChangeText={setQuery}
                placeholder="Search model types"
                placeholderTextColor={m3.outline}
                style={styles.searchInput}
                autoCorrect={false}
              />
            </View>
            {recipes.map((r) => (
              <Pressable
                key={r.id}
                onPress={() => pickRecipe(r)}
                style={({ pressed }) => [styles.recipe, pressed && { backgroundColor: m3.pressed }]}
                accessibilityRole="button"
              >
                <View style={styles.grow}>
                  <Text style={type.bodyLarge}>{r.name}</Text>
                  <Text style={type.bodySmall} numberOfLines={1}>
                    {r.edit ? 'Can edit pictures · ' : ''}
                    {r.slots.filter((s) => s.required).map((s) => s.label).join(', ')}
                  </Text>
                </View>
                <Icon name="chevron-right" size={20} />
              </Pressable>
            ))}
          </>
        ) : (
          <>
            <Pressable
              onPress={() => (state?.model ? undefined : setRecipeId(null))}
              style={styles.recipeHead}
              accessibilityRole="button"
              disabled={!!state?.model}
            >
              <View style={styles.grow}>
                <Text style={type.titleMedium}>{recipe.name}</Text>
                {recipe.description ? (
                  <Text style={[type.bodySmall, styles.muted]} numberOfLines={3}>
                    {recipe.description}
                  </Text>
                ) : null}
              </View>
              {state?.model ? null : <Text style={[type.labelLarge, { color: m3.primary }]}>Change</Text>}
            </Pressable>
            {recipe.model_url ? (
              <Pressable onPress={() => Linking.openURL(recipe.model_url!)} style={styles.link} accessibilityRole="link">
                <Icon name="open-in-new" size={16} color={m3.primary} />
                <Text style={[type.labelLarge, { color: m3.primary }]}>Where to download it</Text>
              </Pressable>
            ) : null}
            <Field label="Name" value={name} onChangeText={setName} autoCapitalize="sentences" />
            {recipe.slots.map((slot) => {
              const cands = catalog?.files.filter((f) => slot.categories.includes(f.category)) ?? [];
              const chosen = catalog?.files.find((f) => f.id === files[slot.id]);
              const open = slotOpen === slot.id;
              const either = recipe.required_any.find((g) => g.includes(slot.id));
              return (
                <View key={slot.id} style={styles.slot}>
                  <Pressable
                    onPress={() => setSlotOpen(open ? null : slot.id)}
                    style={styles.slotHead}
                    accessibilityRole="button"
                    accessibilityLabel={`${slot.label}: ${chosen?.name ?? 'none'}`}
                  >
                    <View style={styles.grow}>
                      <Text style={type.labelLarge}>
                        {slot.label}
                        {slot.required ? '' : either ? ` (or ${either.filter((x) => x !== slot.id).join(', ').toUpperCase()})` : ' (optional)'}
                      </Text>
                      <Text
                        style={[type.bodyMedium, { color: chosen ? m3.onSurface : slot.required ? m3.error : m3.outline }]}
                        numberOfLines={1}
                        ellipsizeMode="middle"
                      >
                        {chosen?.name ?? (cands.length ? 'Choose a file' : `No files in ${slot.categories.join(' or ')}`)}
                      </Text>
                    </View>
                    <Icon name={open ? 'chevron-up' : 'chevron-down'} size={20} />
                  </Pressable>
                  {open ? (
                    <View style={styles.options}>
                      {slot.required ? null : (
                        <Option
                          label="None"
                          on={!files[slot.id]}
                          onPress={() => {
                            const next = { ...files };
                            delete next[slot.id];
                            setFiles(next);
                            setSlotOpen(null);
                          }}
                        />
                      )}
                      {cands.map((f) => (
                        <Option
                          key={f.id}
                          label={f.name}
                          on={files[slot.id] === f.id}
                          onPress={() => {
                            setFiles({ ...files, [slot.id]: f.id });
                            setSlotOpen(null);
                          }}
                        />
                      ))}
                    </View>
                  ) : null}
                </View>
              );
            })}
          </>
        )}
        {error ? <Banner message={error} /> : null}
      </ScrollView>
      <View style={styles.sheetActions}>
        <Button title="Cancel" variant="text" onPress={onClose} />
        {recipe ? (
          <Button
            title="Save"
            loading={busy}
            disabled={!name.trim() || !setupComplete(recipe, files)}
            onPress={async () => {
              setBusy(true);
              try {
                await onSave(name.trim(), recipe.id, files);
              } catch (e) {
                setError(describeError(e));
              } finally {
                setBusy(false);
              }
            }}
          />
        ) : null}
      </View>
    </Sheet>
  );
}

function Option({ label, on, onPress }: { label: string; on: boolean; onPress: () => void }) {
  return (
    <Pressable
      onPress={onPress}
      style={({ pressed }) => [styles.option, on && styles.optionOn, pressed && { backgroundColor: m3.pressed }]}
      accessibilityRole="radio"
      accessibilityState={{ checked: on }}
    >
      <Text style={[type.bodyMedium, styles.grow, on && { color: m3.onSecondaryContainer }]} numberOfLines={1} ellipsizeMode="middle">
        {label}
      </Text>
      {on ? <Icon name="check" size={18} color={m3.primary} /> : null}
    </Pressable>
  );
}

const styles = StyleSheet.create({
  content: { paddingHorizontal: space.lg, paddingTop: space.sm, gap: space.sm },
  grow: { flex: 1, minWidth: 0 },
  muted: { color: m3.onSurfaceVariant },
  note: { color: m3.onSurfaceVariant, marginHorizontal: space.sm, marginBottom: space.sm, lineHeight: 19 },
  engineRow: { flexDirection: 'row', alignItems: 'center', gap: space.lg, paddingHorizontal: space.lg, paddingVertical: space.md },
  engineIcon: {
    width: 44,
    height: 44,
    borderRadius: 14,
    backgroundColor: m3.surfaceContainerHigh,
    alignItems: 'center',
    justifyContent: 'center',
  },
  track: { height: 4, borderRadius: 2, backgroundColor: m3.surfaceContainerHighest, marginTop: space.sm, overflow: 'hidden' },
  fill: { height: 4, backgroundColor: m3.primary },
  modelRow: { flexDirection: 'row', alignItems: 'center', paddingRight: space.xs },
  modelMain: {
    flex: 1,
    minWidth: 0,
    flexDirection: 'row',
    alignItems: 'center',
    gap: space.lg,
    paddingHorizontal: space.lg,
    minHeight: 64,
    paddingVertical: space.sm,
  },
  buttons: { flexDirection: 'row', flexWrap: 'wrap', gap: space.sm, marginVertical: space.sm },
  loadingRow: { flexDirection: 'row', alignItems: 'center', gap: space.md, padding: space.md },
  search: {
    flexDirection: 'row',
    alignItems: 'center',
    gap: space.sm,
    height: 44,
    paddingHorizontal: space.md,
    borderRadius: 22,
    backgroundColor: m3.surfaceContainerHigh,
    marginBottom: space.sm,
  },
  searchInput: { flex: 1, color: m3.onSurface, fontSize: 15, height: 44 },
  recipe: { flexDirection: 'row', alignItems: 'center', gap: space.md, minHeight: 56, paddingHorizontal: space.sm, borderRadius: radius.md },
  recipeHead: { flexDirection: 'row', alignItems: 'flex-start', gap: space.md, marginBottom: space.sm },
  link: { flexDirection: 'row', alignItems: 'center', gap: 6, marginBottom: space.md },
  slot: { borderRadius: radius.md, backgroundColor: m3.surfaceContainerHigh, marginBottom: space.sm, overflow: 'hidden' },
  slotHead: { flexDirection: 'row', alignItems: 'center', gap: space.md, padding: space.md },
  options: { paddingHorizontal: space.xs, paddingBottom: space.xs },
  option: { flexDirection: 'row', alignItems: 'center', gap: space.sm, minHeight: 40, paddingHorizontal: space.md, borderRadius: radius.sm },
  optionOn: { backgroundColor: m3.secondaryContainer },
  sheetActions: { flexDirection: 'row', justifyContent: 'flex-end', gap: space.sm, marginTop: space.md },
});
