/**
 * Things shared to Relay from other apps through the phone's share sheet: a
 * link or some text, screenshots and photos, voice notes or any other file.
 * On Android the native module (modules/relay-share) receives them and copies
 * the files into the app's cache; the share screen (app/share.tsx) then lets
 * you pick a chat and send them.
 */
import { requireOptionalNativeModule } from 'expo';
import { Platform } from 'react-native';

export type SharedFile = {
  name: string;
  mime: string;
  size: number;
  /** A file:// path in the app's cache; missing when it couldn't be copied. */
  uri?: string;
  error?: 'too_big' | 'unreadable';
};

export type SharedPayload = {
  /** Counts shares, so a new one replaces the one on screen. */
  id: number;
  text: string | null;
  subject: string | null;
  files: SharedFile[];
  /** Files left out because too many were shared at once. */
  dropped: number;
};

type RelayShare = {
  take(): Promise<Omit<SharedPayload, 'id'> | null>;
  addListener(event: 'onShare', listener: () => void): { remove(): void };
};

const native = Platform.OS === 'android' ? requireOptionalNativeModule<RelayShare>('RelayShare') : null;

/** Whether this build can receive shares from other apps. */
export const shareAvailable = !!native;

let pending: SharedPayload | null = null;
let count = 0;

/** Reads the share that opened Relay (or arrived while it was open), once. */
export async function takeShare(): Promise<SharedPayload | null> {
  if (!native) return null;
  try {
    const raw = await native.take();
    if (!raw || (!raw.text?.trim() && !raw.subject?.trim() && raw.files.length === 0)) return null;
    pending = { ...raw, id: ++count };
    return pending;
  } catch {
    return null;
  }
}

/** The share waiting on the share screen. */
export function pendingShare(): SharedPayload | null {
  return pending;
}

export function clearShare() {
  pending = null;
}

/** Calls `f` whenever something is shared to Relay while it's open. */
export function onShare(f: () => void): () => void {
  if (!native) return () => undefined;
  const sub = native.addListener('onShare', f);
  return () => sub.remove();
}

/**
 * The prompt text for a share: the shared text, with the page title or
 * subject on its own line above it when the text doesn't already have it
 * (browsers share a link as the text and the page title as the subject).
 */
export function sharedText(p: SharedPayload): string {
  const text = p.text?.trim() ?? '';
  const subject = p.subject?.trim() ?? '';
  if (!subject || text.includes(subject)) return text;
  return text ? `${subject}\n${text}` : subject;
}

/** True when the shared text is just a link. */
export function isLink(text: string): boolean {
  return /^https?:\/\/\S+$/i.test(text.trim());
}
