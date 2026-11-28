/**
 * Hands-free voice mode for a chat: listen, send, wait for the agent, read the
 * reply aloud, and listen again. Approvals are asked and answered by voice
 * too. Speech is recognized and spoken by the phone itself (see lib/voice).
 */
import { useKeepAwake } from 'expo-keep-awake';
import { useEffect, useRef, useState } from 'react';
import {
  ActivityIndicator,
  Animated,
  AppState,
  Easing,
  Linking,
  Modal,
  Platform,
  Pressable,
  ScrollView,
  StyleSheet,
  Text,
  View,
} from 'react-native';
import { useSafeAreaInsets } from 'react-native-safe-area-context';

import type { ApprovalDecision, Message, PendingApproval, SessionStatus } from '../lib/types';
import {
  describeApproval,
  describeHeardError,
  downloadOfflineSpeech,
  isExitPhrase,
  listen,
  loadVoicePrefs,
  type Listening,
  parseApproval,
  prepareVoice,
  type Readiness,
  saveVoicePrefs,
  speak,
  speakable,
  type Speaking,
  type SpeechRate,
  type VoicePrefs,
} from '../lib/voice';
import { m3, radius, space, type } from './theme';
import { Button, Icon, IconButton, type IconName } from './ui';

type Next = 'listen' | 'ask' | 'work' | 'close' | 'pause';

type Phase =
  | { kind: 'preparing' }
  | { kind: 'blocked'; readiness: Extract<Readiness, { ok: false }>; note?: string }
  | { kind: 'listening' }
  | { kind: 'sending'; text: string }
  | { kind: 'working' }
  | { kind: 'speaking'; text: string; next: Next }
  /** Listening for "allow" or "deny". */
  | { kind: 'asking' }
  | { kind: 'paused'; note?: string };

/** Times in a row nothing was heard before voice mode waits for a tap. */
const MAX_SILENCES = 2;

export interface VoiceModeProps {
  visible: boolean;
  onClose: () => void;
  /** Sends what was said; resolves to the stored message's id, or null if it failed. */
  onSend: (text: string) => Promise<number | null>;
  onStop: () => void;
  onDecide: (decision: ApprovalDecision) => Promise<void>;
  status: SessionStatus;
  messages: Message[];
  approval: PendingApproval | null;
  taskError: string | null;
  /** What the agent is writing right now. */
  draftText: string;
  /** A short label for what the agent is doing (e.g. "Running a command"). */
  activity: string | null;
  plain: boolean;
}

export function VoiceMode(props: VoiceModeProps) {
  if (!props.visible) return null;
  return <VoiceSession {...props} />;
}

function VoiceSession({
  onClose,
  onSend,
  onStop,
  onDecide,
  status,
  messages,
  approval,
  taskError,
  draftText,
  activity,
  plain,
}: VoiceModeProps) {
  useKeepAwake('relay-voice');
  const insets = useSafeAreaInsets();
  const [prefs, setPrefs] = useState<VoicePrefs>(loadVoicePrefs);
  const [phase, setPhase] = useState<Phase>({ kind: 'preparing' });
  const [heard, setHeard] = useState('');
  const [onDevice, setOnDevice] = useState(true);
  const [busy, setBusy] = useState(false);
  const level = useRef(new Animated.Value(0)).current;

  const running = status === 'running';
  const alive = useRef(true);
  const listening = useRef<Listening | null>(null);
  const speaking = useRef<Speaking | null>(null);
  /** The user message this turn's reply follows. */
  const sentId = useRef<number | null>(null);
  const silences = useRef(0);
  const askTries = useRef(0);
  /** Approvals already read out, so each is announced once. */
  const announced = useRef(new Set<string>());
  const latest = useRef({ messages, status, taskError, approval, prefs, onDevice });
  latest.current = { messages, status, taskError, approval, prefs, onDevice };

  const go = (p: Phase) => {
    if (alive.current) setPhase(p);
  };
  const say = (text: string, next: Next) => go({ kind: 'speaking', text, next });

  const halt = () => {
    listening.current?.cancel();
    listening.current = null;
    speaking.current?.stop();
    speaking.current = null;
  };

  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
      halt();
    };
  }, []);

  // Get the microphone and recognizer ready.
  const start = async (allowOnline = prefs.allowOnline) => {
    go({ kind: 'preparing' });
    const r = await prepareVoice(allowOnline);
    if (!alive.current) return;
    if (!r.ok) {
      go({ kind: 'blocked', readiness: r });
      return;
    }
    setOnDevice(r.onDevice);
    latest.current.onDevice = r.onDevice;
    silences.current = 0;
    if (latest.current.status === 'running') {
      // Joining a task that's already going: its reply follows the last thing sent.
      const lastUser = [...latest.current.messages].reverse().find((m) => m.role === 'user');
      sentId.current = lastUser?.id ?? null;
      go({ kind: 'working' });
    } else {
      go({ kind: 'listening' });
    }
  };
  useEffect(() => {
    void start();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Phones don't let apps listen in the background: pause, and wait for a tap.
  useEffect(() => {
    const sub = AppState.addEventListener('change', (st) => {
      if (st === 'active') return;
      halt();
      setPhase((p) =>
        p.kind === 'listening' || p.kind === 'speaking' || p.kind === 'asking' ? { kind: 'paused' } : p,
      );
    });
    return () => sub.remove();
  }, []);

  // Listening for a prompt.
  useEffect(() => {
    if (phase.kind !== 'listening') return;
    setHeard('');
    const l = listen({
      onDevice: latest.current.onDevice,
      onPartial: setHeard,
      onLevel: (v) => Animated.timing(level, { toValue: v, duration: 110, useNativeDriver: true }).start(),
    });
    listening.current = l;
    void l.done.then(async (h) => {
      if (listening.current === l) listening.current = null;
      if (!alive.current || h.kind === 'cancelled') return;
      if (h.kind === 'silence') {
        silences.current += 1;
        if (silences.current >= MAX_SILENCES) go({ kind: 'paused' });
        else go({ kind: 'listening' });
        return;
      }
      if (h.kind === 'error') {
        if (h.code === 'language-not-supported' && latest.current.onDevice) {
          go({
            kind: 'blocked',
            readiness: { ok: false, reason: 'needs-download', message: describeHeardError(h.code, h.message) },
          });
        } else {
          go({ kind: 'paused', note: describeHeardError(h.code, h.message) });
        }
        return;
      }
      silences.current = 0;
      setHeard(h.text);
      if (isExitPhrase(h.text)) {
        say('Okay. Bye for now.', 'close');
        return;
      }
      go({ kind: 'sending', text: h.text });
      const id = await onSend(h.text);
      if (!alive.current) return;
      if (id === null) {
        say("That didn't go through. Check the chat, then tap to try again.", 'pause');
        return;
      }
      sentId.current = id;
      go({ kind: 'working' });
    });
    return () => {
      if (listening.current === l) {
        l.cancel();
        listening.current = null;
      }
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [phase]);

  // While the agent works: announce approvals, and read the reply when it ends.
  useEffect(() => {
    if (phase.kind !== 'working') return;
    if (approval && !announced.current.has(approval.tool_call_id)) {
      announced.current.add(approval.tool_call_id);
      askTries.current = 0;
      say(`${describeApproval(approval)} Say allow, or deny.`, 'ask');
      return;
    }
    if (running) return;
    const after = sentId.current ?? -1;
    const replies = messages.filter((m) => m.role === 'assistant' && m.id > after && m.content.trim());
    const last = replies[replies.length - 1];
    let text = last ? speakable(last.content) : '';
    if (status === 'error') {
      text = `Something went wrong. ${taskError ? speakable(taskError).slice(0, 200) : ''}`.trim();
    } else if (status === 'stopped') {
      text = text ? `Stopped. ${text}` : 'Stopped.';
    } else if (!text) {
      text = plain ? "I don't have an answer for that." : "I'm done. The details are in the chat.";
    }
    say(text, 'listen');
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [phase, running, approval, messages, status]);

  // Reading aloud.
  useEffect(() => {
    if (phase.kind !== 'speaking') return;
    const s = speak(phase.text, latest.current.prefs.rate);
    speaking.current = s;
    const next = phase.next;
    void s.done.then(() => {
      if (speaking.current !== s) return; // Interrupted: whoever stopped it moves on.
      speaking.current = null;
      if (!alive.current) return;
      if (next === 'close') onClose();
      else if (next === 'listen') go({ kind: 'listening' });
      else if (next === 'ask') go({ kind: latest.current.approval ? 'asking' : 'working' });
      else if (next === 'pause') go({ kind: 'paused' });
      else go({ kind: 'working' });
    });
    return () => {
      if (speaking.current === s) {
        speaking.current = null;
        s.stop();
      }
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [phase]);

  // Listening for an answer to an approval.
  useEffect(() => {
    if (phase.kind !== 'asking') return;
    setHeard('');
    const l = listen({
      onDevice: latest.current.onDevice,
      short: true,
      onPartial: setHeard,
      onLevel: (v) => Animated.timing(level, { toValue: v, duration: 110, useNativeDriver: true }).start(),
    });
    listening.current = l;
    void l.done.then(async (h) => {
      if (listening.current === l) listening.current = null;
      if (!alive.current || h.kind === 'cancelled') return;
      if (!latest.current.approval) {
        go({ kind: 'working' });
        return;
      }
      const decision = h.kind === 'text' ? parseApproval(h.text) : null;
      if (!decision) {
        askTries.current += 1;
        if (askTries.current < 2) say('Sorry, I didn\'t catch that. Say allow, or deny.', 'ask');
        // Leave it on screen to tap.
        else go({ kind: 'working' });
        return;
      }
      await decide(decision);
    });
    return () => {
      if (listening.current === l) {
        l.cancel();
        listening.current = null;
      }
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [phase]);

  // Answered somewhere else (the card, another phone): stop asking.
  useEffect(() => {
    if (phase.kind === 'asking' && !approval) {
      halt();
      go({ kind: 'working' });
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [approval, phase.kind]);

  const decide = async (decision: ApprovalDecision) => {
    halt();
    setBusy(true);
    try {
      await onDecide(decision);
    } finally {
      setBusy(false);
    }
    say(decision === 'deny' ? 'Okay, denied.' : decision === 'approve_all' ? 'Allowed for this task.' : 'Allowed.', 'work');
  };

  const talk = () => {
    halt();
    silences.current = 0;
    askTries.current = 0;
    if (latest.current.approval) go({ kind: 'asking' });
    else if (latest.current.status === 'running') go({ kind: 'working' });
    else go({ kind: 'listening' });
  };

  const setRate = (rate: SpeechRate) => {
    const p = { ...prefs, rate };
    setPrefs(p);
    saveVoicePrefs(p);
  };

  const allowOnline = () => {
    const p = { ...prefs, allowOnline: true };
    setPrefs(p);
    saveVoicePrefs(p);
    void start(true);
  };

  // ----- what's shown -----

  const listeningNow = phase.kind === 'listening' || phase.kind === 'asking';
  const view = describe(phase, { running, approval, activity, plain });
  // While an approval waits, what it's for stays on screen.
  const asked =
    approval && (phase.kind === 'working' || phase.kind === 'asking' || phase.kind === 'speaking')
      ? describeApproval(approval)
      : null;
  const caption =
    phase.kind === 'asking'
      ? heard
      : phase.kind === 'speaking'
      ? phase.text
      : phase.kind === 'working'
        ? draftText.trim()
          ? speakable(draftText).slice(-400)
          : heard
        : phase.kind === 'sending'
          ? phase.text
          : heard;

  return (
    <Modal visible animationType="slide" onRequestClose={onClose} statusBarTranslucent>
      <View style={[styles.screen, { paddingTop: insets.top, paddingBottom: Math.max(insets.bottom, space.lg) }]}>
        <View style={styles.top}>
          <IconButton icon="close" label="Leave voice mode" color={m3.onSurface} onPress={onClose} />
          <View style={styles.topText}>
            <Text style={type.titleMedium}>Voice</Text>
            <View style={styles.privacy}>
              <Icon name={onDevice ? 'shield-check-outline' : 'cloud-outline'} size={14} color={m3.onSurfaceVariant} />
              <Text style={type.bodySmall}>
                {onDevice ? 'Speech stays on this phone' : "Using the phone's online speech service"}
              </Text>
            </View>
          </View>
          <RateChip rate={prefs.rate} onChange={setRate} />
        </View>

        <View style={styles.middle}>
          <Orb level={level} icon={view.icon} active={listeningNow} busy={view.busy} />
          <Text style={[type.titleLarge, styles.status]} accessibilityLiveRegion="polite">
            {view.title}
          </Text>
          {view.hint ? <Text style={[type.bodyMedium, styles.hint]}>{view.hint}</Text> : null}
        </View>

        <ScrollView style={styles.captionBox} contentContainerStyle={styles.captionInner}>
          {phase.kind === 'blocked' ? (
            <Text style={[type.bodyLarge, styles.caption]}>{phase.note ?? phase.readiness.message}</Text>
          ) : phase.kind === 'paused' && phase.note ? (
            <Text style={[type.bodyLarge, styles.caption]}>{phase.note}</Text>
          ) : asked ? (
            <>
              <Text style={[type.bodyLarge, styles.caption, styles.captionReply]}>{asked}</Text>
              {phase.kind === 'asking' && heard ? (
                <Text style={[type.bodyMedium, styles.caption, styles.heard]}>“{heard}”</Text>
              ) : null}
            </>
          ) : caption ? (
            <Text style={[type.bodyLarge, styles.caption, phase.kind === 'speaking' && styles.captionReply]}>
              {caption}
            </Text>
          ) : null}
        </ScrollView>

        <View style={styles.controls}>
          {phase.kind === 'blocked' ? (
            <BlockedActions
              readiness={phase.readiness}
              onRetry={() => void start()}
              onAllowOnline={allowOnline}
              onDownload={async () => {
                const note = await downloadOfflineSpeech();
                if (alive.current) go({ kind: 'blocked', readiness: phase.readiness, note });
              }}
            />
          ) : approval && (phase.kind === 'working' || phase.kind === 'asking' || phase.kind === 'speaking') ? (
            <View style={styles.row}>
              <Button title="Deny" icon="close" variant="outlined" onPress={() => void decide('deny')} disabled={busy} />
              <Button title="Allow" icon="check" onPress={() => void decide('approve')} disabled={busy} />
            </View>
          ) : (
            <View style={styles.row}>
              {phase.kind === 'working' || phase.kind === 'sending' ? (
                <Button title="Stop the agent" icon="stop" variant="outlined" onPress={onStop} />
              ) : phase.kind === 'listening' ? (
                <>
                  <Button
                    title="Pause"
                    icon="pause"
                    variant="outlined"
                    onPress={() => {
                      halt();
                      go({ kind: 'paused' });
                    }}
                  />
                  <Button title="Send now" icon="arrow-up" onPress={() => listening.current?.finish()} />
                </>
              ) : phase.kind === 'speaking' ? (
                <Button title="Skip, I'll talk" icon="microphone" onPress={talk} />
              ) : phase.kind === 'paused' ? (
                <Button title="Talk" icon="microphone" onPress={talk} />
              ) : null}
            </View>
          )}
          <Text style={[type.bodySmall, styles.tip]}>
            {phase.kind === 'listening'
              ? 'Say "goodbye" to leave voice mode.'
              : phase.kind === 'speaking'
                ? 'Everything is also in the chat.'
                : ' '}
          </Text>
        </View>
      </View>
    </Modal>
  );
}

function describe(
  phase: Phase,
  ctx: { running: boolean; approval: PendingApproval | null; activity: string | null; plain: boolean },
): { title: string; hint?: string; icon: IconName; busy?: boolean } {
  switch (phase.kind) {
    case 'preparing':
      return { title: 'Getting ready…', icon: 'microphone-outline', busy: true };
    case 'blocked':
      return { title: "Voice mode can't start", icon: 'microphone-off' };
    case 'listening':
      return { title: 'Listening…', hint: 'Talk, then pause when you’re done.', icon: 'microphone' };
    case 'sending':
      return { title: 'Sending…', icon: 'arrow-up', busy: true };
    case 'working':
      if (ctx.approval) return { title: 'Needs your approval', hint: 'Tap Allow or Deny.', icon: 'shield-alert-outline' };
      return {
        title: ctx.activity ?? (ctx.plain ? 'Thinking…' : 'Working on it…'),
        icon: 'dots-horizontal',
        busy: true,
      };
    case 'speaking':
      if (ctx.approval)
        return { title: 'Needs your approval', hint: 'Answer out loud, or tap Allow or Deny.', icon: 'volume-high' };
      return { title: 'Speaking…', hint: 'Tap Skip to talk instead.', icon: 'volume-high' };
    case 'asking':
      return { title: 'Allow or deny?', hint: 'Say "allow" or "deny".', icon: 'microphone' };
    case 'paused':
      return { title: 'Paused', hint: 'Tap Talk when you’re ready.', icon: 'pause' };
  }
}

function Orb({
  level,
  icon,
  active,
  busy,
}: {
  level: Animated.Value;
  icon: IconName;
  active: boolean;
  busy?: boolean;
}) {
  const pulse = useRef(new Animated.Value(0)).current;
  useEffect(() => {
    if (!busy) {
      pulse.setValue(0);
      return;
    }
    const loop = Animated.loop(
      Animated.sequence([
        Animated.timing(pulse, { toValue: 1, duration: 900, easing: Easing.inOut(Easing.quad), useNativeDriver: true }),
        Animated.timing(pulse, { toValue: 0, duration: 900, easing: Easing.inOut(Easing.quad), useNativeDriver: true }),
      ]),
    );
    loop.start();
    return () => loop.stop();
  }, [busy, pulse]);
  const ring = active
    ? level.interpolate({ inputRange: [0, 1], outputRange: [1, 1.45] })
    : pulse.interpolate({ inputRange: [0, 1], outputRange: [1, 1.12] });
  return (
    <View style={styles.orbWrap}>
      <Animated.View
        style={[
          styles.ring,
          { transform: [{ scale: ring }], opacity: active ? 0.35 : busy ? 0.2 : 0.12 },
        ]}
      />
      <View style={[styles.orb, active && { backgroundColor: m3.primary }]}>
        {busy && icon === 'dots-horizontal' ? (
          <ActivityIndicator color={m3.onPrimaryContainer} />
        ) : (
          <Icon name={icon} size={44} color={active ? m3.onPrimary : m3.onPrimaryContainer} />
        )}
      </View>
    </View>
  );
}

function RateChip({ rate, onChange }: { rate: SpeechRate; onChange: (r: SpeechRate) => void }) {
  const order: SpeechRate[] = ['slow', 'normal', 'fast'];
  const label = { slow: 'Slower', normal: 'Normal', fast: 'Faster' }[rate];
  return (
    <Pressable
      onPress={() => onChange(order[(order.indexOf(rate) + 1) % order.length])}
      style={({ pressed }) => [styles.chip, pressed && { backgroundColor: m3.surfaceContainerHighest }]}
      accessibilityRole="button"
      accessibilityLabel={`Reading speed: ${label}. Tap to change.`}
      hitSlop={6}
    >
      <Icon name="speedometer" size={16} color={m3.onSurfaceVariant} />
      <Text style={type.labelLarge}>{label}</Text>
    </Pressable>
  );
}

function BlockedActions({
  readiness,
  onRetry,
  onAllowOnline,
  onDownload,
}: {
  readiness: Extract<Readiness, { ok: false }>;
  onRetry: () => void;
  onAllowOnline: () => void;
  onDownload: () => void;
}) {
  return (
    <View style={styles.blocked}>
      {readiness.reason === 'needs-download' && Platform.OS === 'android' ? (
        <Button title="Download offline speech" icon="download" onPress={onDownload} />
      ) : null}
      {readiness.reason === 'permission' ? (
        <Button title="Open phone settings" icon="cog-outline" onPress={() => void Linking.openSettings()} />
      ) : null}
      {readiness.reason === 'no-offline' || readiness.reason === 'needs-download' ? (
        <Button title="Use online speech" icon="cloud-outline" variant="outlined" onPress={onAllowOnline} />
      ) : null}
      <Button title="Try again" icon="refresh" variant="text" onPress={onRetry} />
    </View>
  );
}

const styles = StyleSheet.create({
  screen: { flex: 1, backgroundColor: m3.surface },
  top: { flexDirection: 'row', alignItems: 'center', gap: space.xs, paddingHorizontal: space.xs, height: 64 },
  topText: { flex: 1, minWidth: 0 },
  privacy: { flexDirection: 'row', alignItems: 'center', gap: 4 },
  chip: {
    flexDirection: 'row',
    alignItems: 'center',
    gap: 6,
    height: 32,
    paddingHorizontal: space.md,
    marginRight: space.sm,
    borderRadius: radius.sm,
    borderWidth: 1,
    borderColor: m3.outlineVariant,
  },
  middle: { alignItems: 'center', paddingTop: space.xl, gap: space.sm },
  orbWrap: { width: 200, height: 200, alignItems: 'center', justifyContent: 'center', marginBottom: space.lg },
  ring: { position: 'absolute', width: 160, height: 160, borderRadius: 80, backgroundColor: m3.primary },
  orb: {
    width: 128,
    height: 128,
    borderRadius: 64,
    backgroundColor: m3.primaryContainer,
    alignItems: 'center',
    justifyContent: 'center',
  },
  status: { textAlign: 'center' },
  hint: { textAlign: 'center', color: m3.onSurfaceVariant },
  captionBox: { flex: 1, marginTop: space.lg },
  captionInner: { paddingHorizontal: space.xl, paddingBottom: space.md },
  caption: { textAlign: 'center', color: m3.onSurfaceVariant },
  captionReply: { color: m3.onSurface },
  heard: { marginTop: space.md, fontStyle: 'italic' },
  controls: { paddingHorizontal: space.lg, gap: space.sm, alignItems: 'center' },
  row: { flexDirection: 'row', gap: space.md, justifyContent: 'center', minHeight: 48, alignItems: 'center' },
  blocked: { gap: space.sm, alignItems: 'center' },
  tip: { textAlign: 'center', color: m3.outline },
});
