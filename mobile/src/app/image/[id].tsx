import { router, useFocusEffect, useLocalSearchParams } from 'expo-router';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import {
  ActivityIndicator,
  Image,
  KeyboardAvoidingView,
  Platform,
  Pressable,
  ScrollView,
  StyleSheet,
  Text,
  TextInput,
  View,
} from 'react-native';
import { useSafeAreaInsets } from 'react-native-safe-area-context';

import { useAttachments } from '../../components/Attachments';
import { alert } from '../../components/Dialogs';
import { ImageModelChip, ImageModelSheet, PictureViewer, StoredImage } from '../../components/ImageBits';
import { Sheet } from '../../components/Sheet';
import { m3, radius, space, type } from '../../components/theme';
import { Banner, Button, Field, Icon, IconButton, type IconName } from '../../components/ui';
import { describeError } from '../../lib/api';
import { useClient } from '../../lib/connection';
import { emit } from '../../lib/events';
import {
  imagesApi,
  SIZES,
  stageLabel,
  type ImageChat,
  type ImageModel,
  type ImageProject,
  type ImagesStatus,
  type ImageTurn,
  type LiveTurn,
} from '../../lib/images';
import { enterToSend, useWide } from '../../lib/layout';

const POLL_BUSY_MS = 800;
const POLL_IDLE_MS = 6000;
const FEED_MAX = 860;

/** Settings under the tune button; empty means "the model's default". */
type Tune = { steps: string; cfg: string; strength: string; seed: string; negative: string };
const NO_TUNE: Tune = { steps: '', cfg: '', strength: '', seed: '', negative: '' };

/**
 * An image chat: each message makes pictures, and a message can start from
 * the last picture to keep developing it. Laid out like a chat: your prompt
 * on the right, the pictures below it.
 */
export default function ImageChatScreen() {
  const client = useClient();
  const api = useMemo(() => (client ? imagesApi(client) : null), [client]);
  const { id, title: initialTitle } = useLocalSearchParams<{ id: string; title?: string }>();
  const insets = useSafeAreaInsets();
  const wide = useWide();

  const [chat, setChat] = useState<ImageChat | null>(null);
  const [project, setProject] = useState<ImageProject | null>(null);
  const [turns, setTurns] = useState<ImageTurn[] | null>(null);
  const [live, setLive] = useState<Record<string, LiveTurn>>({});
  const [models, setModels] = useState<ImageModel[]>([]);
  const [status, setStatus] = useState<ImagesStatus | null>(null);
  const [error, setError] = useState<string | null>(null);

  const [text, setText] = useState('');
  const [sending, setSending] = useState(false);
  const [stopping, setStopping] = useState(false);
  /** The picture the next message starts from. */
  const [base, setBase] = useState<string | null>(null);
  const [sizeId, setSizeId] = useState<string | null>(null);
  const [count, setCount] = useState(1);
  const [tune, setTune] = useState<Tune>(NO_TUNE);
  const [tuning, setTuning] = useState(false);
  const [picking, setPicking] = useState(false);
  const [viewing, setViewing] = useState<string | null>(null);
  const upload = useAttachments();
  const uploaded = upload.images[0] ?? null;

  const scroll = useRef<React.ComponentRef<typeof ScrollView>>(null);
  const input = useRef<React.ComponentRef<typeof TextInput>>(null);
  const lastDone = useRef<string | null>(null);

  const running = turns?.find((t) => t.status === 'queued' || t.status === 'running') ?? null;
  const model = models.find((m) => m.id === chat?.model_id) ?? models.find((m) => m.id === project?.model_id) ?? models[models.length - 1];

  const load = useCallback(async () => {
    if (!api || !id) return;
    try {
      const [c, t] = await Promise.all([api.chat(id), api.turns(id)]);
      setChat(c);
      setTurns(t.turns);
      setLive(t.live);
      setError(null);
      if (!project || project.id !== c.project_id) api.project(c.project_id).then(setProject).catch(() => undefined);
    } catch (e) {
      setError(describeError(e));
    }
  }, [api, id, project]);

  useEffect(() => {
    if (!api) return;
    api.models().then(setModels).catch(() => undefined);
    api.status().then(setStatus).catch(() => undefined);
  }, [api]);

  // Opening an image chat gets the GPU ready for pictures.
  useEffect(() => {
    if (api && models.length) api.prepare().catch(() => undefined);
  }, [api, models.length]);

  useFocusEffect(
    useCallback(() => {
      load();
    }, [load]),
  );

  // Poll quickly while a picture is being made.
  useEffect(() => {
    if (!api) return;
    const t = setTimeout(
      () => {
        load();
        api.status().then(setStatus).catch(() => undefined);
      },
      running ? POLL_BUSY_MS : POLL_IDLE_MS,
    );
    return () => clearTimeout(t);
  }, [api, load, running, turns, live]);

  // When a picture finishes, the next message refines it.
  useEffect(() => {
    const done = [...(turns ?? [])].reverse().find((t) => t.status === 'done');
    if (!done || done.id === lastDone.current) return;
    const first = lastDone.current === null;
    lastDone.current = done.id;
    if (!first || !base) setBase(done.images[0] ?? null);
    if (!first) emit('chats');
  }, [turns, base]);

  const scrollToEnd = useCallback(() => {
    setTimeout(() => scroll.current?.scrollToEnd({ animated: true }), 50);
  }, []);

  const send = async (override?: { prompt: string; base: string | null; seed?: number }) => {
    if (!api || !id) return;
    const prompt = (override?.prompt ?? text).trim();
    if (!prompt || sending || running) return;
    setSending(true);
    try {
      const size = SIZES.find((s) => s.id === sizeId);
      const num = (s: string) => (s.trim() === '' ? undefined : Number(s));
      const startFrom = override ? override.base : base;
      const t = await api.generate(id, {
        prompt,
        model_id: model?.id,
        count,
        width: size?.w,
        height: size?.h,
        steps: num(tune.steps),
        cfg_scale: num(tune.cfg),
        strength: startFrom || (!override && uploaded) ? num(tune.strength) : undefined,
        seed: override?.seed ?? num(tune.seed),
        negative: tune.negative.trim() || undefined,
        base_image: startFrom ?? undefined,
        base_upload: !override && !startFrom && uploaded ? uploaded : undefined,
      });
      if (!override) setText('');
      upload.clear();
      setTurns((cur) => [...(cur ?? []), t]);
      emit('chats');
      scrollToEnd();
      load();
    } catch (e) {
      alert("Couldn't make the picture", describeError(e));
    } finally {
      setSending(false);
    }
  };

  const stop = async () => {
    if (!api || !running) return;
    setStopping(true);
    try {
      await api.stopTurn(running.id);
      await load();
    } finally {
      setStopping(false);
    }
  };

  const removeTurn = (t: ImageTurn) =>
    alert('Delete these pictures?', 'They are removed from Relay.', [
      { text: 'Cancel', style: 'cancel' },
      {
        text: 'Delete',
        style: 'destructive',
        onPress: async () => {
          try {
            await api?.deleteTurn(t.id);
            if (t.images.includes(base ?? '')) setBase(null);
            load();
          } catch (e) {
            alert("Couldn't delete", describeError(e));
          }
        },
      },
    ]);

  const engine = status?.engine;
  const engineLine =
    engine?.state === 'setup'
      ? (engine.message ?? 'Downloading the image engine…')
      : engine?.state === 'error'
        ? (engine.message ?? 'The image engine has a problem')
        : null;
  const title = chat?.title ?? initialTitle ?? 'Image';
  const sizeLabel = SIZES.find((s) => s.id === sizeId)?.label ?? 'Size';
  const tuned = Object.values(tune).some((v) => v.trim() !== '');

  return (
    <View style={[styles.flex, { paddingTop: insets.top }]}>
      <View style={[styles.header, wide && styles.headerWide]}>
        {wide ? null : (
          <IconButton
            icon="arrow-left"
            label="Back"
            color={m3.onSurface}
            onPress={() => (router.canGoBack() ? router.back() : router.replace('/'))}
          />
        )}
        <View style={styles.headerText}>
          <Text style={type.titleMedium} numberOfLines={1}>
            {title}
          </Text>
          {project ? (
            <Pressable
              onPress={() =>
                router.push({ pathname: '/images/[id]', params: { id: project.id, name: project.name } })
              }
              hitSlop={6}
              style={styles.projectRow}
              accessibilityRole="link"
            >
              <Icon name="image-multiple-outline" size={14} color={m3.onSurfaceVariant} />
              <Text style={[type.bodySmall, styles.muted]} numberOfLines={1}>
                {project.name}
              </Text>
            </Pressable>
          ) : null}
        </View>
        <GpuBadge status={status} busy={!!running} />
        <View style={{ width: space.sm }} />
      </View>
      {error ? <Banner message={error} actionLabel="Retry" onAction={load} /> : null}
      {engineLine ? (
        <Banner
          kind={engine?.state === 'error' ? 'error' : 'info'}
          message={engineLine}
          actionLabel={engine?.state === 'error' ? 'Image models' : undefined}
          onAction={engine?.state === 'error' ? () => router.push('/image-models') : undefined}
        />
      ) : null}

      <KeyboardAvoidingView style={styles.flex} behavior={Platform.OS === 'ios' ? 'padding' : undefined}>
        <ScrollView
          ref={scroll}
          style={styles.flex}
          contentContainerStyle={[styles.feed, wide && styles.feedWide]}
          onContentSizeChange={() => scroll.current?.scrollToEnd({ animated: false })}
          keyboardShouldPersistTaps="handled"
        >
          {turns === null ? (
            <ActivityIndicator color={m3.primary} style={{ marginTop: space.xxl }} />
          ) : turns.length === 0 ? (
            <EmptyChat noModels={models.length === 0} />
          ) : (
            turns.map((t, i) => (
              <TurnView
                key={t.id}
                turn={t}
                live={live[t.id]}
                latest={i === turns.length - 1}
                wide={wide}
                selected={base}
                onOpen={setViewing}
                onRefine={(name) => {
                  setBase(name);
                  input.current?.focus();
                }}
                onAgain={() => send({ prompt: t.prompt, base: t.base_image })}
                onUsePrompt={() => {
                  setText(t.prompt);
                  input.current?.focus();
                }}
                onDelete={() => removeTurn(t)}
              />
            ))
          )}
        </ScrollView>

        <View style={[styles.composerWrap, wide && styles.feedWide, { paddingBottom: Math.max(insets.bottom, wide ? space.lg : space.sm) }]}>
          <View style={styles.box}>
            {base || uploaded ? (
              <View style={styles.baseRow}>
                <View style={styles.baseThumb}>
                  {base ? (
                    <StoredImage name={base} style={StyleSheet.absoluteFill} />
                  ) : uploaded ? (
                    <Image source={{ uri: uploaded }} style={StyleSheet.absoluteFill} />
                  ) : null}
                </View>
                <View style={styles.grow}>
                  <Text style={type.labelLarge}>{base ? 'Refining this picture' : 'Starting from your picture'}</Text>
                  <Text style={[type.bodySmall, styles.muted]} numberOfLines={2}>
                    {modelCanEdit(model)
                      ? 'Say what to change, like "make it night" or "add a hat".'
                      : 'Describe what to add or change. The picture is redrawn from this one.'}
                  </Text>
                </View>
                <IconButton
                  icon="close"
                  size={20}
                  label="Start a new picture instead"
                  onPress={() => {
                    setBase(null);
                    upload.clear();
                  }}
                />
              </View>
            ) : null}
            {upload.error ? <Text style={[type.bodySmall, styles.err]}>{upload.error}</Text> : null}
            <TextInput
              ref={input}
              style={styles.input}
              value={text}
              onChangeText={setText}
              onKeyPress={enterToSend(() => send())}
              placeholder={
                models.length === 0
                  ? 'Set up an image model first…'
                  : base || uploaded
                    ? 'What should change?'
                    : 'Describe a picture…'
              }
              placeholderTextColor={m3.outline}
              selectionColor={m3.primary}
              cursorColor={m3.primary}
              multiline
              autoCapitalize="sentences"
              autoFocus={wide}
              accessibilityLabel="Picture prompt"
            />
            <View style={styles.toolRow}>
              <ScrollView horizontal showsHorizontalScrollIndicator={false} contentContainerStyle={styles.tools} style={styles.grow}>
                <IconButton
                  icon="image-plus"
                  size={22}
                  label="Start from a picture of yours"
                  onPress={() => {
                    setBase(null);
                    upload.clear();
                    upload.pick('library');
                  }}
                  disabled={sending}
                />
                <ImageModelChip label={model?.name ?? 'No image model'} onPress={() => setPicking(true)} />
                <ToolChip
                  icon={(SIZES.find((s) => s.id === sizeId)?.icon as IconName) ?? 'aspect-ratio'}
                  label={sizeLabel}
                  onPress={() => {
                    const i = SIZES.findIndex((s) => s.id === sizeId);
                    setSizeId(i === SIZES.length - 1 ? null : SIZES[i + 1].id);
                  }}
                  hint="Picture shape. Tap to change; Size uses the model's default (or the picture you start from)."
                />
                <ToolChip
                  icon="image-multiple-outline"
                  label={`×${count}`}
                  onPress={() => setCount((c) => (c % 4) + 1)}
                  hint="How many pictures to make. Tap to change."
                />
                <ToolChip
                  icon="tune-variant"
                  label={tuned ? 'Tuned' : 'Tune'}
                  active={tuned}
                  onPress={() => setTuning(true)}
                  hint="Steps, guidance, seed and what to leave out"
                />
              </ScrollView>
              {running ? (
                <Pressable
                  onPress={stop}
                  disabled={stopping}
                  style={[styles.round, styles.stop]}
                  accessibilityRole="button"
                  accessibilityLabel="Stop the picture"
                >
                  {stopping ? <ActivityIndicator color={m3.onErrorContainer} /> : <Icon name="stop" size={22} color={m3.onErrorContainer} />}
                </Pressable>
              ) : (
                <Pressable
                  onPress={() => send()}
                  disabled={!text.trim() || sending || models.length === 0}
                  style={[styles.round, { backgroundColor: text.trim() && models.length ? m3.primary : m3.surfaceContainerHighest }]}
                  accessibilityRole="button"
                  accessibilityLabel="Make the picture"
                >
                  {sending ? (
                    <ActivityIndicator color={m3.onPrimary} />
                  ) : (
                    <Icon name="arrow-up" size={22} color={text.trim() && models.length ? m3.onPrimary : m3.outline} />
                  )}
                </Pressable>
              )}
            </View>
          </View>
        </View>
      </KeyboardAvoidingView>

      <ImageModelSheet
        visible={picking}
        models={models}
        value={model?.id}
        onClose={() => setPicking(false)}
        onPick={async (mid) => {
          if (!api || !id) return;
          setChat(await api.updateChat(id, { model_id: mid }));
          setPicking(false);
        }}
      />
      <TuneSheet visible={tuning} value={tune} onChange={setTune} onClose={() => setTuning(false)} />
      <PictureViewer
        name={viewing}
        onClose={() => setViewing(null)}
        actions={
          viewing
            ? [
                {
                  icon: 'auto-fix',
                  label: 'Refine this',
                  onPress: () => {
                    setBase(viewing);
                    input.current?.focus();
                  },
                },
              ]
            : undefined
        }
      />
    </View>
  );
}

/** Editing model families (their recipe ids say so: kontext, *-edit, pix2pix). */
function modelCanEdit(model: ImageModel | undefined): boolean {
  return !!model && /kontext|edit|pix2pix/i.test(model.recipe);
}

function GpuBadge({ status, busy }: { status: ImagesStatus | null; busy: boolean }) {
  if (!status) return null;
  const e = status.engine;
  const loaded = e.gpu === 'images' && e.state === 'ready';
  const label = busy ? 'Drawing' : loaded ? 'Image model on GPU' : status.chat_model === 'ready' ? 'Chat model on GPU' : 'GPU free';
  const color = busy || loaded ? m3.primary : m3.onSurfaceVariant;
  return (
    <View style={styles.badge} accessibilityLabel={label}>
      <Icon name="expansion-card-variant" size={14} color={color} />
      <Text style={[type.labelMedium, { color }]} numberOfLines={1}>
        {label}
      </Text>
    </View>
  );
}

function EmptyChat({ noModels }: { noModels: boolean }) {
  return (
    <View style={styles.empty}>
      <View style={styles.spark}>
        <Icon name="palette-outline" size={28} color={m3.primary} />
      </View>
      <Text style={[type.headlineMedium, styles.center]}>What should we draw?</Text>
      <Text style={[type.bodyLarge, styles.center, styles.muted]}>
        Describe a picture. Each message after that refines the last picture, so you can keep developing it. Pictures
        are made on your PC; the chat model steps aside while they are.
      </Text>
      {noModels ? (
        <Button title="Set up an image model" icon="cog-outline" onPress={() => router.push('/image-models')} />
      ) : null}
    </View>
  );
}

function TurnView({
  turn,
  live,
  latest,
  wide,
  selected,
  onOpen,
  onRefine,
  onAgain,
  onUsePrompt,
  onDelete,
}: {
  turn: ImageTurn;
  live: LiveTurn | undefined;
  latest: boolean;
  wide: boolean;
  selected: string | null;
  onOpen: (name: string) => void;
  onRefine: (name: string) => void;
  onAgain: () => void;
  onUsePrompt: () => void;
  onDelete: () => void;
}) {
  const busy = turn.status === 'queued' || turn.status === 'running';
  const w = turn.params.width ?? 1024;
  const h = turn.params.height ?? 1024;
  const ratio = Math.min(2, Math.max(0.5, w / h));
  const many = turn.images.length > 1 || (busy && turn.params.count > 1);
  const cell = many ? styles.cellHalf : wide ? styles.cellWide : styles.cellOne;
  // Keep a tall picture from filling the whole window on a desktop.
  const capped = wide && !many ? { maxWidth: Math.round(600 * ratio) } : null;
  const progress = live?.step != null && live.total ? live.step / live.total : null;

  return (
    <View style={styles.turn}>
      <View style={styles.userRow}>
        {turn.base_image ? (
          <Pressable onPress={() => onOpen(turn.base_image!)} style={styles.fromThumb} accessibilityLabel="The picture this started from">
            <StoredImage name={turn.base_image} style={StyleSheet.absoluteFill} />
            <View style={styles.fromBadge}>
              <Icon name="auto-fix" size={12} color="#fff" />
            </View>
          </Pressable>
        ) : null}
        <Pressable onLongPress={onUsePrompt} style={styles.bubble} accessibilityHint="Long press to use this prompt again">
          <Text style={type.bodyLarge}>{turn.prompt}</Text>
        </Pressable>
      </View>

      {busy ? (
        <View style={[styles.grid]}>
          <View style={[cell, capped, styles.pending, { aspectRatio: ratio }]}>
            {live?.preview ? (
              <Image source={{ uri: live.preview }} style={[StyleSheet.absoluteFill, styles.previewImg]} blurRadius={2} />
            ) : (
              <View style={[StyleSheet.absoluteFill, styles.shimmer]} />
            )}
            <View style={styles.pendingInner}>
              <ActivityIndicator color={m3.onSurface} />
              <Text style={[type.labelLarge, styles.pendingText]}>{stageLabel(live, turn.status)}</Text>
            </View>
            <View style={styles.progressTrack}>
              <View style={[styles.progressFill, { width: `${Math.round((progress ?? 0) * 100)}%` }]} />
            </View>
          </View>
        </View>
      ) : turn.status === 'done' ? (
        <View style={styles.grid}>
          {turn.images.map((name) => (
            <Pressable
              key={name}
              onPress={() => onOpen(name)}
              onLongPress={() => onRefine(name)}
              style={[cell, capped, styles.picture, { aspectRatio: ratio }, selected === name && styles.pictureSelected]}
              accessibilityRole="imagebutton"
              accessibilityLabel="Open picture"
              accessibilityHint="Long press to refine this picture"
            >
              <StoredImage name={name} style={StyleSheet.absoluteFill} />
              {selected === name ? (
                <View style={styles.selectedTag}>
                  <Icon name="auto-fix" size={14} color={m3.onPrimary} />
                  <Text style={[type.labelMedium, { color: m3.onPrimary }]}>Refining</Text>
                </View>
              ) : null}
            </Pressable>
          ))}
        </View>
      ) : (
        <View style={[styles.failed, turn.status === 'stopped' && styles.stopped]}>
          <Icon
            name={turn.status === 'stopped' ? 'stop-circle-outline' : 'alert-circle-outline'}
            size={20}
            color={turn.status === 'stopped' ? m3.onSurfaceVariant : m3.onErrorContainer}
          />
          <Text style={[type.bodyMedium, styles.grow, { color: turn.status === 'stopped' ? m3.onSurfaceVariant : m3.onErrorContainer }]}>
            {turn.status === 'stopped' ? 'Stopped.' : (turn.error ?? 'The picture failed.')}
          </Text>
        </View>
      )}

      {!busy ? (
        <View style={styles.metaRow}>
          <Text style={[type.bodySmall, styles.muted, styles.grow]} numberOfLines={1}>
            {[
              turn.model_name,
              turn.params.width && turn.params.height ? `${turn.params.width}×${turn.params.height}` : null,
              turn.seed != null ? `seed ${turn.seed}` : null,
              turn.duration_ms ? `${(turn.duration_ms / 1000).toFixed(1)} s` : null,
            ]
              .filter(Boolean)
              .join(' · ')}
          </Text>
          {turn.status === 'done' && turn.images[0] ? (
            <MetaButton icon="auto-fix" label="Refine" onPress={() => onRefine(turn.images[0])} />
          ) : null}
          {latest ? <MetaButton icon="refresh" label="Again" onPress={onAgain} /> : null}
          <MetaButton icon="trash-can-outline" label="Delete" onPress={onDelete} />
        </View>
      ) : null}
      {turn.full_prompt !== turn.prompt && !busy ? (
        <Text style={[type.bodySmall, styles.fullPrompt]} numberOfLines={3}>
          Prompt used: {turn.full_prompt}
        </Text>
      ) : null}
    </View>
  );
}

function MetaButton({ icon, label, onPress }: { icon: IconName; label: string; onPress: () => void }) {
  return (
    <Pressable
      onPress={onPress}
      hitSlop={4}
      style={({ pressed, hovered }: { pressed: boolean; hovered?: boolean }) => [styles.metaButton, (pressed || hovered) && styles.hover]}
      accessibilityRole="button"
      accessibilityLabel={label}
    >
      <Icon name={icon} size={16} color={m3.onSurfaceVariant} />
      <Text style={[type.labelMedium, styles.muted]}>{label}</Text>
    </Pressable>
  );
}

function ToolChip({
  icon,
  label,
  onPress,
  hint,
  active,
}: {
  icon: IconName;
  label: string;
  onPress: () => void;
  hint: string;
  active?: boolean;
}) {
  return (
    <Pressable
      onPress={onPress}
      style={({ pressed, hovered }: { pressed: boolean; hovered?: boolean }) => [
        styles.chip,
        active && styles.chipActive,
        (pressed || hovered) && !active && styles.hover,
      ]}
      accessibilityRole="button"
      accessibilityLabel={label}
      accessibilityHint={hint}
    >
      <Icon name={icon} size={16} color={active ? m3.onSecondaryContainer : m3.onSurfaceVariant} />
      <Text style={[type.labelLarge, { color: active ? m3.onSecondaryContainer : m3.onSurfaceVariant }]}>{label}</Text>
    </Pressable>
  );
}

function TuneSheet({
  visible,
  value,
  onChange,
  onClose,
}: {
  visible: boolean;
  value: Tune;
  onChange: (t: Tune) => void;
  onClose: () => void;
}) {
  const [t, setT] = useState(value);
  useEffect(() => {
    if (visible) setT(value);
  }, [visible, value]);
  const set = (k: keyof Tune) => (v: string) => setT((cur) => ({ ...cur, [k]: v.replace(/[^0-9.]/g, '') }));
  return (
    <Sheet visible={visible} onClose={onClose} title="Tune">
      <ScrollView style={{ maxHeight: 460 }} keyboardShouldPersistTaps="handled">
        <Text style={[type.bodyMedium, styles.muted, { marginBottom: space.md }]}>
          Leave a box empty to use the model&apos;s own setting.
        </Text>
        <View style={styles.tuneRow}>
          <View style={styles.grow}>
            <Field label="Steps" value={t.steps} onChangeText={set('steps')} keyboardType="number-pad" placeholder="Model default" />
          </View>
          <View style={styles.grow}>
            <Field label="Guidance (CFG)" value={t.cfg} onChangeText={set('cfg')} keyboardType="decimal-pad" placeholder="Model default" />
          </View>
        </View>
        <View style={styles.tuneRow}>
          <View style={styles.grow}>
            <Field
              label="Change when refining (0–1)"
              value={t.strength}
              onChangeText={set('strength')}
              keyboardType="decimal-pad"
              placeholder="0.6"
            />
          </View>
          <View style={styles.grow}>
            <Field label="Seed" value={t.seed} onChangeText={set('seed')} keyboardType="number-pad" placeholder="Random" />
          </View>
        </View>
        <Field
          label="Leave out"
          value={t.negative}
          onChangeText={(v) => setT((cur) => ({ ...cur, negative: v }))}
          placeholder="blurry, text, watermark"
          autoCapitalize="none"
          hint="Added to the project's own list"
        />
      </ScrollView>
      <View style={styles.sheetActions}>
        <Button title="Reset" variant="text" onPress={() => setT(NO_TUNE)} />
        <Button
          title="Done"
          onPress={() => {
            onChange(t);
            onClose();
          }}
        />
      </View>
    </Sheet>
  );
}

const styles = StyleSheet.create({
  flex: { flex: 1, backgroundColor: m3.surface },
  grow: { flex: 1, minWidth: 0 },
  muted: { color: m3.onSurfaceVariant },
  center: { textAlign: 'center' },
  err: { color: m3.error, paddingHorizontal: space.md, paddingTop: space.xs },
  header: { height: 64, flexDirection: 'row', alignItems: 'center', paddingHorizontal: space.xs, gap: space.xs },
  headerWide: { paddingLeft: space.xl, paddingRight: space.lg, borderBottomWidth: 1, borderBottomColor: m3.outlineVariant },
  headerText: { flex: 1, minWidth: 0, paddingHorizontal: space.xs },
  projectRow: { flexDirection: 'row', alignItems: 'center', gap: 4, alignSelf: 'flex-start' },
  badge: {
    flexDirection: 'row',
    alignItems: 'center',
    gap: 6,
    height: 28,
    paddingHorizontal: space.md,
    borderRadius: 14,
    backgroundColor: m3.surfaceContainerHigh,
    maxWidth: 190,
  },
  feed: { paddingHorizontal: space.lg, paddingTop: space.lg, paddingBottom: space.md, flexGrow: 1 },
  feedWide: { width: '100%', maxWidth: FEED_MAX, alignSelf: 'center' },
  empty: { flex: 1, justifyContent: 'center', alignItems: 'center', gap: space.md, paddingHorizontal: space.lg, paddingVertical: space.xxl },
  spark: {
    width: 56,
    height: 56,
    borderRadius: 18,
    backgroundColor: m3.surfaceContainerHigh,
    alignItems: 'center',
    justifyContent: 'center',
  },
  turn: { marginBottom: space.xl, gap: space.sm },
  userRow: { flexDirection: 'row', justifyContent: 'flex-end', alignItems: 'flex-end', gap: space.sm, marginLeft: 48 },
  bubble: {
    flexShrink: 1,
    backgroundColor: m3.surfaceContainerHigh,
    borderRadius: 20,
    paddingHorizontal: space.lg,
    paddingVertical: space.md,
  },
  fromThumb: { width: 44, height: 44, borderRadius: radius.sm, overflow: 'hidden', backgroundColor: m3.surfaceContainerHigh },
  fromBadge: {
    position: 'absolute',
    right: 2,
    bottom: 2,
    width: 18,
    height: 18,
    borderRadius: 9,
    backgroundColor: 'rgba(0,0,0,0.6)',
    alignItems: 'center',
    justifyContent: 'center',
  },
  grid: { flexDirection: 'row', flexWrap: 'wrap', gap: space.sm },
  cellOne: { width: '100%' },
  cellWide: { width: '78%' },
  cellHalf: { width: '48.5%' },
  picture: { borderRadius: radius.lg, overflow: 'hidden', backgroundColor: m3.surfaceContainerHigh },
  pictureSelected: { borderWidth: 2, borderColor: m3.primary },
  selectedTag: {
    position: 'absolute',
    left: space.sm,
    top: space.sm,
    flexDirection: 'row',
    alignItems: 'center',
    gap: 4,
    paddingHorizontal: space.sm,
    height: 24,
    borderRadius: 12,
    backgroundColor: m3.primary,
  },
  pending: {
    borderRadius: radius.lg,
    overflow: 'hidden',
    backgroundColor: m3.surfaceContainerHigh,
    alignItems: 'center',
    justifyContent: 'center',
  },
  previewImg: { opacity: 0.85 },
  shimmer: { backgroundColor: m3.surfaceContainerHighest, opacity: 0.4 },
  pendingInner: {
    flexDirection: 'row',
    alignItems: 'center',
    gap: space.sm,
    paddingHorizontal: space.lg,
    height: 40,
    borderRadius: 20,
    backgroundColor: 'rgba(0,0,0,0.55)',
  },
  pendingText: { color: '#fff' },
  progressTrack: { position: 'absolute', left: 0, right: 0, bottom: 0, height: 4, backgroundColor: 'rgba(255,255,255,0.12)' },
  progressFill: { height: 4, backgroundColor: m3.primary },
  failed: {
    flexDirection: 'row',
    alignItems: 'center',
    gap: space.md,
    padding: space.md,
    borderRadius: radius.lg,
    backgroundColor: m3.errorContainer,
  },
  stopped: { backgroundColor: m3.surfaceContainerHigh },
  metaRow: { flexDirection: 'row', alignItems: 'center', gap: 2 },
  metaButton: { flexDirection: 'row', alignItems: 'center', gap: 4, height: 32, paddingHorizontal: space.sm, borderRadius: 16 },
  hover: { backgroundColor: m3.hover },
  fullPrompt: { color: m3.outline },
  composerWrap: { paddingHorizontal: space.md, paddingTop: space.xs },
  box: { backgroundColor: m3.surfaceContainerHigh, borderRadius: radius.xl, paddingVertical: space.sm, paddingHorizontal: space.sm },
  baseRow: {
    flexDirection: 'row',
    alignItems: 'center',
    gap: space.md,
    margin: space.xs,
    marginBottom: 0,
    padding: space.sm,
    borderRadius: radius.lg,
    backgroundColor: m3.surfaceContainerHighest,
  },
  baseThumb: { width: 48, height: 48, borderRadius: radius.sm, overflow: 'hidden', backgroundColor: m3.surfaceContainer },
  input: {
    minHeight: 44,
    maxHeight: 180,
    color: m3.onSurface,
    fontSize: 16,
    lineHeight: 22,
    paddingHorizontal: space.md,
    paddingTop: space.sm,
    paddingBottom: space.xs,
  },
  toolRow: { flexDirection: 'row', alignItems: 'center', gap: space.sm },
  tools: { flexDirection: 'row', alignItems: 'center', gap: 2 },
  chip: { flexDirection: 'row', alignItems: 'center', gap: 6, height: 36, paddingHorizontal: space.md, borderRadius: 18 },
  chipActive: { backgroundColor: m3.secondaryContainer },
  round: { width: 44, height: 44, borderRadius: 22, alignItems: 'center', justifyContent: 'center' },
  stop: { borderRadius: 14, backgroundColor: m3.errorContainer },
  tuneRow: { flexDirection: 'row', gap: space.md },
  sheetActions: { flexDirection: 'row', justifyContent: 'flex-end', gap: space.sm, marginTop: space.md },
});
