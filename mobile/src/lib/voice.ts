/**
 * Voice mode: hands-free talk with the agent. Speech is turned into text and
 * replies are read aloud by the phone itself (Android's and iOS's own speech
 * recognizer and text-to-speech), so nothing goes to a cloud service. When the
 * phone has no offline recognizer, the person can choose to allow the phone's
 * online one.
 *
 * The recognizer is a native module that older builds of the app don't have,
 * so it's loaded lazily and voice mode simply doesn't show without it.
 */
import * as Secure from 'expo-secure-store';
import * as Speech from 'expo-speech';
import { Platform } from 'react-native';

import type { ApprovalDecision, PendingApproval } from './types';

type RecognitionModule = typeof import('expo-speech-recognition').ExpoSpeechRecognitionModule;

let recognition: RecognitionModule | null | undefined;

function recognizer(): RecognitionModule | null {
  if (recognition !== undefined) return recognition;
  recognition = null;
  // The browser's speech recognition sends audio to a cloud service, so the
  // desktop and web builds don't offer voice mode.
  if (Platform.OS === 'web') return null;
  try {
    // eslint-disable-next-line @typescript-eslint/no-require-imports
    recognition = (require('expo-speech-recognition') as typeof import('expo-speech-recognition'))
      .ExpoSpeechRecognitionModule;
  } catch {
    recognition = null;
  }
  return recognition;
}

/** This build of the app can do voice mode. */
export function voiceAvailable(): boolean {
  return recognizer() !== null;
}

// ----- preferences -----

export type SpeechRate = 'slow' | 'normal' | 'fast';

export interface VoicePrefs {
  /** Speaking speed for replies. */
  rate: SpeechRate;
  /** Use the phone's online recognizer when it has no offline one. */
  allowOnline: boolean;
}

const PREFS_KEY = 'relay.voice';
const DEFAULT_PREFS: VoicePrefs = { rate: 'normal', allowOnline: false };

export function loadVoicePrefs(): VoicePrefs {
  try {
    const raw = Secure.getItem(PREFS_KEY);
    if (!raw) return DEFAULT_PREFS;
    const p = JSON.parse(raw) as Partial<VoicePrefs>;
    return {
      rate: p.rate === 'slow' || p.rate === 'fast' ? p.rate : 'normal',
      allowOnline: p.allowOnline === true,
    };
  } catch {
    return DEFAULT_PREFS;
  }
}

export function saveVoicePrefs(prefs: VoicePrefs): void {
  try {
    Secure.setItem(PREFS_KEY, JSON.stringify(prefs));
  } catch {
    // Not saved; it still applies until the app closes.
  }
}

const RATES: Record<SpeechRate, number> = { slow: 0.85, normal: 1.0, fast: 1.2 };

// ----- getting ready -----

/** The phone's language, e.g. "en-US". */
export function speechLanguage(): string {
  try {
    const loc = Intl.DateTimeFormat().resolvedOptions().locale;
    if (loc && /^[a-z]{2,3}(-[A-Za-z0-9]+)*$/.test(loc)) {
      // Recognizers want language-REGION; "en" alone is often refused.
      return loc.includes('-') ? loc : loc === 'en' ? 'en-US' : loc;
    }
  } catch {
    // Fall through.
  }
  return 'en-US';
}

export type Readiness =
  | { ok: true; onDevice: boolean }
  | {
      ok: false;
      reason: 'unavailable' | 'permission' | 'no-offline' | 'needs-download';
      message: string;
    };

/**
 * Checks the microphone permission and whether speech can be recognized on
 * the phone itself. `allowOnline` accepts the phone's online recognizer.
 */
export async function prepareVoice(allowOnline: boolean): Promise<Readiness> {
  const mod = recognizer();
  if (!mod) {
    return { ok: false, reason: 'unavailable', message: 'This version of the app has no voice mode. Update the app.' };
  }
  if (!mod.isRecognitionAvailable()) {
    return {
      ok: false,
      reason: 'unavailable',
      message:
        Platform.OS === 'ios'
          ? 'Speech recognition is off on this iPhone. Turn on Siri & Dictation in Settings, then try again.'
          : 'This phone has no speech recognition service. Install or turn on Speech Recognition & Synthesis from Google, then try again.',
    };
  }
  const perm = await mod.requestPermissionsAsync();
  if (!perm.granted) {
    return {
      ok: false,
      reason: 'permission',
      message: 'Relay needs the microphone to hear you. Allow it in the phone settings, then try again.',
    };
  }
  if (!mod.supportsOnDeviceRecognition()) {
    if (allowOnline) return { ok: true, onDevice: false };
    return {
      ok: false,
      reason: 'no-offline',
      message:
        "This phone can't recognize speech offline. You can let it use the phone's online speech service instead; your words then go to that service (Google or Apple) to be turned into text.",
    };
  }
  if (Platform.OS === 'android') {
    // On-device recognition on Android needs the language pack installed.
    try {
      const lang = speechLanguage();
      const { installedLocales } = await mod.getSupportedLocales({});
      const have = installedLocales.some((l) => l.toLowerCase() === lang.toLowerCase());
      const sameLanguage = installedLocales.some((l) => l.split('-')[0] === lang.split('-')[0]);
      if (installedLocales.length && !have && !sameLanguage) {
        if (allowOnline) return { ok: true, onDevice: false };
        return {
          ok: false,
          reason: 'needs-download',
          message: `Offline speech for ${lang} isn't downloaded on this phone yet.`,
        };
      }
    } catch {
      // Unknown: try anyway, and report what the recognizer says.
    }
  }
  return { ok: true, onDevice: true };
}

/** Android: asks the phone to download the offline speech pack for its language. */
export async function downloadOfflineSpeech(): Promise<string> {
  const mod = recognizer();
  if (!mod) return 'Voice mode is not available in this version of the app.';
  try {
    const r = await mod.androidTriggerOfflineModelDownload({ locale: speechLanguage() });
    if (r.status === 'download_success') return 'Downloaded. You can talk now.';
    if (r.status === 'download_scheduled') return 'The download is queued (it may wait for Wi-Fi). Try again when it finishes.';
    return 'Finish the download in the dialog, then try again.';
  } catch (e) {
    return `The download didn't start: ${e instanceof Error ? e.message : String(e)}`;
  }
}

// ----- listening -----

export type Heard =
  | { kind: 'text'; text: string }
  /** Nothing was said. */
  | { kind: 'silence' }
  | { kind: 'cancelled' }
  | { kind: 'error'; code: string; message: string };

export interface Listening {
  done: Promise<Heard>;
  /** Stop listening and use what was heard so far. */
  finish: () => void;
  /** Stop listening and drop it. */
  cancel: () => void;
}

/**
 * Listens for one thing said. `onPartial` gets the words as they're
 * recognized and `onLevel` the input level (0..1) for the animation.
 */
export function listen(opts: {
  onDevice: boolean;
  onPartial?: (text: string) => void;
  onLevel?: (level: number) => void;
  /** Short answers such as "allow" or "deny". */
  short?: boolean;
}): Listening {
  const mod = recognizer();
  if (!mod) {
    return {
      done: Promise.resolve({ kind: 'error', code: 'unavailable', message: 'Voice mode is not available.' }),
      finish: () => undefined,
      cancel: () => undefined,
    };
  }
  let finalText = '';
  let partial = '';
  let error: { code: string; message: string } | null = null;
  let cancelled = false;
  let settle: (h: Heard) => void = () => undefined;
  const done = new Promise<Heard>((resolve) => {
    settle = resolve;
  });
  const subs = [
    mod.addListener('result', (ev) => {
      const text = ev.results[0]?.transcript?.trim() ?? '';
      if (ev.isFinal) {
        // Continuous results arrive as separate final pieces.
        finalText = [finalText, text].filter(Boolean).join(' ');
        partial = '';
      } else {
        partial = text;
      }
      opts.onPartial?.([finalText, partial].filter(Boolean).join(' '));
    }),
    mod.addListener('error', (ev) => {
      error = { code: ev.error, message: ev.message };
    }),
    mod.addListener('volumechange', (ev) => {
      // Android reports about -2..10, iOS a similar range.
      opts.onLevel?.(Math.max(0, Math.min(1, (ev.value + 2) / 12)));
    }),
    mod.addListener('end', () => {
      for (const s of subs) s.remove();
      opts.onLevel?.(0);
      const text = [finalText, partial].filter(Boolean).join(' ').trim();
      if (cancelled) settle({ kind: 'cancelled' });
      else if (text) settle({ kind: 'text', text });
      else if (!error || error.code === 'no-speech' || error.code === 'speech-timeout' || error.code === 'aborted')
        settle({ kind: 'silence' });
      else settle({ kind: 'error', ...error });
    }),
  ];
  try {
    mod.start({
      lang: speechLanguage(),
      interimResults: true,
      maxAlternatives: 1,
      continuous: false,
      requiresOnDeviceRecognition: opts.onDevice,
      addsPunctuation: true,
      iosTaskHint: opts.short ? 'confirmation' : 'dictation',
      androidIntentOptions: {
        // Wait for a real pause before ending the turn, so a breath in the
        // middle of a sentence doesn't send half of it.
        EXTRA_SPEECH_INPUT_COMPLETE_SILENCE_LENGTH_MILLIS: opts.short ? 900 : 1800,
        EXTRA_SPEECH_INPUT_POSSIBLY_COMPLETE_SILENCE_LENGTH_MILLIS: opts.short ? 700 : 1500,
        ...(opts.short ? { EXTRA_LANGUAGE_MODEL: 'web_search' } : {}),
      },
      volumeChangeEventOptions: { enabled: true, intervalMillis: 120 },
    });
  } catch (e) {
    for (const s of subs) s.remove();
    settle({ kind: 'error', code: 'start', message: e instanceof Error ? e.message : String(e) });
  }
  return {
    done,
    finish: () => mod.stop(),
    cancel: () => {
      cancelled = true;
      mod.abort();
    },
  };
}

/** What a recognizer error means, in words for the person. */
export function describeHeardError(code: string, message: string): string {
  switch (code) {
    case 'not-allowed':
      return 'Relay needs the microphone to hear you. Allow it in the phone settings.';
    case 'language-not-supported':
      return `Speech for ${speechLanguage()} isn't available offline on this phone.`;
    case 'network':
      return "The phone's speech service needs the internet and couldn't reach it.";
    case 'audio-capture':
      return 'The microphone is busy or unavailable.';
    case 'interrupted':
      return 'Listening was interrupted (a call or another app).';
    case 'busy':
      return 'The speech recognizer is busy. Try again in a moment.';
    case 'service-not-allowed':
      return 'Speech recognition is turned off on this phone.';
    default:
      return message || `Speech recognition failed (${code}).`;
  }
}

// ----- speaking -----

export interface Speaking {
  done: Promise<void>;
  stop: () => void;
}

/** Reads text aloud; `done` settles when it finishes or is stopped. */
export function speak(text: string, rate: SpeechRate): Speaking {
  const chunks = splitForSpeech(text, Math.min(Speech.maxSpeechInputLength || 4000, 1500));
  let stopped = false;
  const done = new Promise<void>((resolve) => {
    if (!chunks.length) {
      resolve();
      return;
    }
    // Chunks are queued one after another; the last one ends it.
    chunks.forEach((chunk, i) => {
      const last = i === chunks.length - 1;
      Speech.speak(chunk, {
        language: speechLanguage(),
        rate: RATES[rate],
        onDone: last ? resolve : undefined,
        onStopped: resolve,
        onError: () => resolve(),
      });
    });
  });
  return {
    done,
    stop: () => {
      if (stopped) return;
      stopped = true;
      void Speech.stop();
    },
  };
}

/** Splits text at sentence ends into pieces the speech engine accepts. */
export function splitForSpeech(text: string, max: number): string[] {
  const sentences = text.match(/[^.!?\n]+[.!?]*\s*|\n+/g) ?? [text];
  const out: string[] = [];
  let cur = '';
  for (const s of sentences) {
    if (cur && cur.length + s.length > max) {
      out.push(cur.trim());
      cur = '';
    }
    if (s.length > max) {
      for (let i = 0; i < s.length; i += max) out.push(s.slice(i, i + max).trim());
    } else {
      cur += s;
    }
  }
  if (cur.trim()) out.push(cur.trim());
  return out.filter(Boolean);
}

/**
 * Turns a Markdown reply into something good to listen to: no symbols, code
 * or links read out, and a pointer to the chat for what was left out.
 */
export function speakable(markdown: string): string {
  let t = markdown;
  // Relay's own note on a cut-off reply.
  t = t.replace(/\n+---\n+\*Relay:[\s\S]*$/, '');
  let skipped = false;
  t = t.replace(/```[\s\S]*?(```|$)/g, () => {
    skipped = true;
    return '\n';
  });
  t = t.replace(/<think>[\s\S]*?<\/think>/g, '');
  t = t.replace(/!\[[^\]]*\]\([^)]*\)/g, '');
  t = t.replace(/\[([^\]]+)\]\([^)]*\)/g, '$1');
  t = t.replace(/<?https?:\/\/[^\s)>]+>?/g, () => {
    skipped = true;
    return 'a link';
  });
  t = t.replace(/`([^`\n]+)`/g, '$1');
  // Tables: drop the divider rows and read cells as a list.
  t = t.replace(/^[ \t]*\|?[ \t]*:?-{2,}:?[ \t]*(\|[ \t]*:?-{2,}:?[ \t]*)*\|?[ \t]*$/gm, '');
  t = t.replace(/^[ \t]*\|(.*)\|[ \t]*$/gm, (_m, row: string) =>
    row
      .split('|')
      .map((c) => c.trim())
      .filter(Boolean)
      .join(', ')
      .concat('.'),
  );
  t = t.replace(/^[ \t]{0,3}#{1,6}[ \t]+(.*)$/gm, (_m, h: string) => (/[.!?:]$/.test(h) ? h : `${h}.`));
  t = t.replace(/^[ \t]*(?:[-*+]|\d+[.)])[ \t]+(.*)$/gm, (_m, item: string) => (/[.!?:;,]$/.test(item) ? item : `${item}.`));
  t = t.replace(/^[ \t]*>[ \t]?/gm, '');
  t = t.replace(/^[ \t]*(-{3,}|\*{3,}|_{3,})[ \t]*$/gm, '');
  t = t.replace(/(\*\*|__)(.+?)\1/g, '$2');
  t = t.replace(/(^|[\s(])[*_]([^*_\n]+)[*_](?=[\s).,!?:;]|$)/g, '$1$2');
  t = t.replace(/[*#`|~]/g, '');
  t = t.replace(/[ \t]+/g, ' ');
  t = t.replace(/\n{2,}/g, '\n').trim();
  if (skipped) t = `${t}${t ? '\n' : ''}The details are in the chat.`;
  return t;
}

// ----- understanding short answers -----

const EXIT = /^(?:ok(?:ay)?[, ]+)?(?:stop|end|exit|quit|close|leave|turn off)(?: the)? voice(?: mode)?\.?$|^(?:that'?s all|that is all|goodbye|bye|bye bye|i'?m done|we'?re done|that'?s it)[.!]?$/i;

/** "Goodbye", "stop voice mode" and the like end voice mode. */
export function isExitPhrase(text: string): boolean {
  return EXIT.test(text.trim().replace(/[.!?]+$/, ''));
}

/** Reads a spoken answer to an approval: allow, allow everything, or deny. */
export function parseApproval(text: string): ApprovalDecision | null {
  const t = text.toLowerCase().replace(/[^a-z' ]/g, ' ');
  const yes = /\b(allow|approve|yes|yeah|yep|sure|ok|okay|go ahead|do it|run it|fine|proceed|accept)\b/.test(t);
  // "Don't", "deny" and the like win; a plain "no" only counts on its own
  // (so "yes, no problem" still allows).
  if (/\b(don'?t|do not|deny|nope|stop|cancel|reject|decline|never|not allowed)\b/.test(t)) return 'deny';
  if (/\bno\b/.test(t) && !yes) return 'deny';
  if (yes && /\b(everything|all|always)\b/.test(t)) return 'approve_all';
  return yes ? 'approve' : null;
}

/** One sentence that says what the agent wants to do, for reading aloud. */
export function describeApproval(a: PendingApproval): string {
  let args: Record<string, unknown> = {};
  try {
    args = JSON.parse(a.arguments) as Record<string, unknown>;
  } catch {
    // Fall back to the tool name.
  }
  const str = (k: string) => (typeof args[k] === 'string' ? (args[k] as string) : '');
  const file = (p: string) => p.split(/[\\/]/).filter(Boolean).pop() ?? p;
  switch (a.name) {
    case 'run_command': {
      const cmd = str('command');
      const short = cmd.length > 120 ? `${cmd.slice(0, 117)}…` : cmd;
      return short ? `The agent wants to run a command: ${short}.` : 'The agent wants to run a command.';
    }
    case 'write_file':
      return `The agent wants to write the file ${file(str('path')) || 'a file'}.`;
    case 'edit_file':
      return `The agent wants to change ${file(str('path')) || 'a file'}.`;
    default:
      return `The agent wants to use ${a.name.replace(/_/g, ' ')}.`;
  }
}
