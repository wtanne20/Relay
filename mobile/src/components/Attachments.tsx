import * as DocumentPicker from 'expo-document-picker';
import * as FileSystem from 'expo-file-system/legacy';
import * as ImagePicker from 'expo-image-picker';
import { type ComponentRef, useCallback, useEffect, useRef, useState } from 'react';
import { Image, Modal, Platform, Pressable, ScrollView, StyleSheet, Text, View } from 'react-native';

import type { RelayClient } from '../lib/api';
import { useClient } from '../lib/connection';
import { formatBytes } from '../lib/format';
import type { FileRef } from '../lib/types';
import { m3, radius, space, type } from './theme';
import { Icon, IconButton, type IconName } from './ui';

/** Matches the server's limits per prompt. */
export const MAX_PICTURES = 8;
export const MAX_FILES = 8;
export const MAX_FILE_BYTES = 25 * 1024 * 1024;

/** A picture (sent to the model's vision) or any other file (sent as text when it has some). */
export type Attachment =
  | { id: string; kind: 'image'; dataUrl: string }
  | { id: string; kind: 'file'; dataUrl: string; name: string; size: number };

type Incoming = { kind: 'image'; dataUrl: string } | { kind: 'file'; dataUrl: string; name: string; size: number };

let counter = 0;

function readFile(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const r = new FileReader();
    r.onload = () => resolve(String(r.result));
    r.onerror = () => reject(r.error);
    r.readAsDataURL(file);
  });
}

/** Pictures Relay can send to vision; other image types go as files. */
const isPicture = (mime: string) => /^image\/(png|jpe?g|webp|gif|bmp)$/i.test(mime);

async function fromWebFile(f: File): Promise<Incoming> {
  if (f.size > MAX_FILE_BYTES) throw new Error(`${f.name} is larger than 25 MB.`);
  const dataUrl = await readFile(f);
  return isPicture(f.type) ? { kind: 'image', dataUrl } : { kind: 'file', dataUrl, name: f.name || 'file', size: f.size };
}

export function fileIcon(name: string): IconName {
  const ext = name.toLowerCase().split('.').pop() ?? '';
  if (ext === 'pdf') return 'file-pdf-box';
  if (['doc', 'docx', 'odt', 'rtf'].includes(ext)) return 'file-word-outline';
  if (['xls', 'xlsx', 'csv', 'tsv'].includes(ext)) return 'file-table-outline';
  if (['zip', '7z', 'rar', 'gz', 'tar'].includes(ext)) return 'folder-zip-outline';
  if (['txt', 'md', 'log', 'rst'].includes(ext)) return 'file-document-outline';
  if (['png', 'jpg', 'jpeg', 'gif', 'webp', 'svg', 'bmp', 'ico', 'heic'].includes(ext)) return 'file-image-outline';
  if (['mp3', 'wav', 'ogg', 'flac', 'm4a'].includes(ext)) return 'file-music-outline';
  if (['mp4', 'mov', 'webm', 'mkv', 'avi'].includes(ext)) return 'file-video-outline';
  return 'file-code-outline';
}

/**
 * Pictures and files waiting to be sent with the next prompt. On the web and
 * desktop, they can also be pasted or dropped anywhere on the page.
 */
export function useAttachments() {
  const [items, setItems] = useState<Attachment[]>([]);
  const [error, setError] = useState<string | null>(null);

  const add = useCallback((incoming: Incoming[]) => {
    if (incoming.length === 0) return;
    setItems((cur) => {
      const next = [...cur];
      let dropped = false;
      for (const a of incoming) {
        const same = next.filter((x) => x.kind === a.kind).length;
        if (same >= (a.kind === 'image' ? MAX_PICTURES : MAX_FILES)) {
          dropped = true;
          continue;
        }
        next.push({ ...a, id: `a${++counter}` } as Attachment);
      }
      if (dropped) setError(`You can attach up to ${MAX_PICTURES} pictures and ${MAX_FILES} files at a time.`);
      return next;
    });
  }, []);

  const addWebFiles = useCallback(
    (files: File[]) => {
      if (!files.length) return;
      Promise.all(files.map(fromWebFile))
        .then(add)
        .catch((e) => setError(e instanceof Error && e.message ? e.message : "Couldn't read that file."));
    },
    [add],
  );

  useEffect(() => {
    if (Platform.OS !== 'web' || typeof document === 'undefined') return;
    const onPaste = (e: ClipboardEvent) => {
      const files = Array.from(e.clipboardData?.files ?? []);
      if (files.length === 0) return;
      e.preventDefault();
      addWebFiles(files);
    };
    const hasFiles = (e: DragEvent) => Array.from(e.dataTransfer?.types ?? []).includes('Files');
    const onDragOver = (e: DragEvent) => {
      if (hasFiles(e)) e.preventDefault();
    };
    const onDrop = (e: DragEvent) => {
      if (!hasFiles(e)) return;
      e.preventDefault();
      addWebFiles(Array.from(e.dataTransfer?.files ?? []));
    };
    document.addEventListener('paste', onPaste);
    document.addEventListener('dragover', onDragOver);
    document.addEventListener('drop', onDrop);
    return () => {
      document.removeEventListener('paste', onPaste);
      document.removeEventListener('dragover', onDragOver);
      document.removeEventListener('drop', onDrop);
    };
  }, [addWebFiles]);

  const pick = useCallback(
    async (source: AttachSource) => {
      setError(null);
      if (Platform.OS === 'web') {
        const input = document.createElement('input');
        input.type = 'file';
        if (source !== 'file') input.accept = 'image/*';
        input.multiple = true;
        input.onchange = () => addWebFiles(Array.from(input.files ?? []));
        input.click();
        return;
      }
      try {
        if (source === 'file') {
          const res = await DocumentPicker.getDocumentAsync({ multiple: true, copyToCacheDirectory: true, type: '*/*' });
          if (res.canceled) return;
          const out: Incoming[] = [];
          for (const a of res.assets) {
            if (a.size && a.size > MAX_FILE_BYTES) {
              setError(`${a.name} is larger than 25 MB.`);
              continue;
            }
            const b64 = await FileSystem.readAsStringAsync(a.uri, { encoding: FileSystem.EncodingType.Base64 });
            const mime = a.mimeType ?? 'application/octet-stream';
            const dataUrl = `data:${mime};base64,${b64}`;
            out.push(
              isPicture(mime)
                ? { kind: 'image', dataUrl }
                : { kind: 'file', dataUrl, name: a.name || 'file', size: a.size ?? Math.floor((b64.length * 3) / 4) },
            );
          }
          add(out);
          return;
        }
        const options: ImagePicker.ImagePickerOptions = {
          mediaTypes: ['images'],
          base64: true,
          quality: 0.85,
          allowsMultipleSelection: source === 'library',
          selectionLimit: MAX_PICTURES,
        };
        let result: ImagePicker.ImagePickerResult;
        if (source === 'camera') {
          const perm = await ImagePicker.requestCameraPermissionsAsync();
          if (!perm.granted) {
            setError('Relay needs camera access to take a photo. You can allow it in Android settings.');
            return;
          }
          result = await ImagePicker.launchCameraAsync(options);
        } else {
          result = await ImagePicker.launchImageLibraryAsync(options);
        }
        if (result.canceled) return;
        add(
          result.assets
            .filter((a) => a.base64)
            .map((a) => ({ kind: 'image' as const, dataUrl: `data:${a.mimeType ?? 'image/jpeg'};base64,${a.base64}` })),
        );
      } catch {
        setError(source === 'file' ? "Couldn't open that file." : "Couldn't open your pictures.");
      }
    },
    [add, addWebFiles],
  );

  const remove = useCallback((id: string) => setItems((cur) => cur.filter((a) => a.id !== id)), []);
  const clear = useCallback(() => {
    setItems([]);
    setError(null);
  }, []);

  const images = items.filter((a) => a.kind === 'image').map((a) => a.dataUrl);
  const files = items.flatMap((a) => (a.kind === 'file' ? [{ name: a.name, data: a.dataUrl }] : []));
  return { items, images, files, error, setError, pick, remove, clear };
}

export type AttachSource = 'library' | 'camera' | 'file';

/**
 * The composer's + button. It opens a small menu: take a photo and choose
 * photos on a phone, and attach any file everywhere.
 */
export function AttachMenu({ onPick, disabled }: { onPick: (source: AttachSource) => void; disabled?: boolean }) {
  const [open, setOpen] = useState(false);
  const [at, setAt] = useState<{ x: number; y: number } | null>(null);
  const anchor = useRef<ComponentRef<typeof View>>(null);
  const show = () => {
    anchor.current?.measureInWindow((x: number, y: number) => {
      setAt({ x, y });
      setOpen(true);
    });
  };
  const choose = (s: AttachSource) => {
    setOpen(false);
    // Let the menu close before a system picker opens over it.
    setTimeout(() => onPick(s), Platform.OS === 'web' ? 0 : 250);
  };
  const items: { source: AttachSource; icon: IconName; label: string }[] =
    Platform.OS === 'web'
      ? [
          { source: 'library', icon: 'image-outline', label: 'Add photos' },
          { source: 'file', icon: 'paperclip', label: 'Add files' },
        ]
      : [
          { source: 'camera', icon: 'camera-outline', label: 'Take a photo' },
          { source: 'library', icon: 'image-outline', label: 'Choose photos' },
          { source: 'file', icon: 'paperclip', label: 'Attach a file' },
        ];
  return (
    <View ref={anchor} collapsable={false}>
      <IconButton icon="plus" label="Add photos and files" onPress={show} disabled={disabled} size={22} />
      <Modal visible={open} transparent animationType="fade" onRequestClose={() => setOpen(false)}>
        <Pressable style={StyleSheet.absoluteFill} onPress={() => setOpen(false)} accessibilityLabel="Close menu" />
        {at ? (
          <View style={[styles.menu, { left: Math.max(8, at.x - 4), bottom: undefined, top: Math.max(8, at.y - 8 - items.length * 44 - 12) }]}>
            {items.map((it) => (
              <Pressable
                key={it.source}
                onPress={() => choose(it.source)}
                style={({ hovered, pressed }: { pressed: boolean; hovered?: boolean }) => [
                  styles.menuItem,
                  (hovered || pressed) && { backgroundColor: m3.hover },
                ]}
                accessibilityRole="menuitem"
              >
                <Icon name={it.icon} size={20} color={m3.onSurfaceVariant} />
                <Text style={[type.labelLarge, { color: m3.onSurface }]}>{it.label}</Text>
              </Pressable>
            ))}
          </View>
        ) : null}
      </Modal>
    </View>
  );
}

/** A file's name, kind and size as a small chip. */
export function FileChip({ name, size, onRemove }: { name: string; size?: number; onRemove?: () => void }) {
  return (
    <View style={styles.fileChip}>
      <Icon name={fileIcon(name)} size={20} color={m3.primary} />
      <View style={styles.fileText}>
        <Text style={[type.labelLarge, { color: m3.onSurface }]} numberOfLines={1}>
          {name}
        </Text>
        {size !== undefined ? <Text style={[type.bodySmall, { color: m3.outline }]}>{formatSize(size)}</Text> : null}
      </View>
      {onRemove ? (
        <Pressable onPress={onRemove} hitSlop={8} accessibilityRole="button" accessibilityLabel={`Remove ${name}`}>
          <Icon name="close" size={16} color={m3.onSurfaceVariant} />
        </Pressable>
      ) : null}
    </View>
  );
}

function formatSize(n: number): string {
  return n < 1024 ? `${n} bytes` : formatBytes(n);
}

/** Thumbnails and file chips about to be sent, each with a remove button. */
export function AttachmentStrip({ items, onRemove }: { items: Attachment[]; onRemove: (id: string) => void }) {
  if (items.length === 0) return null;
  return (
    <ScrollView horizontal showsHorizontalScrollIndicator={false} contentContainerStyle={styles.strip}>
      {items.map((a) =>
        a.kind === 'file' ? (
          <FileChip key={a.id} name={a.name} size={a.size} onRemove={() => onRemove(a.id)} />
        ) : (
          <View key={a.id} style={styles.thumbWrap}>
            <Image source={{ uri: a.dataUrl }} style={styles.thumb} accessibilityLabel="Attached picture" />
            <Pressable
              onPress={() => onRemove(a.id)}
              style={styles.remove}
              hitSlop={8}
              accessibilityRole="button"
              accessibilityLabel="Remove picture"
            >
              <Icon name="close" size={14} color={m3.onSurface} />
            </Pressable>
          </View>
        ),
      )}
    </ScrollView>
  );
}

/** Files on a sent message. */
export function MessageFiles({ files }: { files: FileRef[] }) {
  if (!files.length) return null;
  return (
    <View style={styles.sentFiles}>
      {files.map((f) => (
        <FileChip key={f.stored} name={f.name} size={f.size} />
      ))}
    </View>
  );
}

/** Loads a stored picture with the token (the web can't send headers with <img>). */
function useAuthedUri(client: RelayClient | null, name: string): string | null {
  const [uri, setUri] = useState<string | null>(null);
  const url = client ? client.attachmentUrl(name) : null;
  const made = useRef<string | null>(null);
  useEffect(() => {
    if (!client || !url) return;
    if (Platform.OS !== 'web') {
      setUri(url);
      return;
    }
    let alive = true;
    fetch(url, { headers: { Authorization: `Bearer ${client.creds.token}` } })
      .then((r) => (r.ok ? r.blob() : Promise.reject(new Error(String(r.status)))))
      .then((b) => {
        if (!alive) return;
        made.current = URL.createObjectURL(b);
        setUri(made.current);
      })
      .catch(() => undefined);
    return () => {
      alive = false;
      if (made.current) URL.revokeObjectURL(made.current);
      made.current = null;
    };
  }, [client, url]);
  return uri;
}

/** A picture in a sent message; tap to see it full size. */
export function MessagePicture({ name }: { name: string }) {
  const client = useClient();
  const uri = useAuthedUri(client, name);
  const [ratio, setRatio] = useState(4 / 3);
  const [open, setOpen] = useState(false);
  useEffect(() => {
    if (!uri || !client) return;
    const done = (w: number, h: number) => {
      if (w && h) setRatio(Math.min(2, Math.max(0.5, w / h)));
    };
    if (Platform.OS === 'web') Image.getSize(uri, done, () => undefined);
    else Image.getSizeWithHeaders(uri, { Authorization: `Bearer ${client.creds.token}` }, done, () => undefined);
  }, [uri, client]);
  const source =
    uri && client
      ? Platform.OS === 'web'
        ? { uri }
        : { uri, headers: { Authorization: `Bearer ${client.creds.token}` } }
      : null;
  return (
    <>
      <Pressable
        onPress={() => setOpen(true)}
        disabled={!source}
        accessibilityRole="imagebutton"
        accessibilityLabel="Open picture"
        style={[styles.picture, { aspectRatio: ratio }]}
      >
        {source ? (
          <Image
            source={source}
            style={StyleSheet.absoluteFill}
            resizeMode="cover"
          />
        ) : null}
      </Pressable>
      <Modal visible={open} transparent animationType="fade" onRequestClose={() => setOpen(false)}>
        <Pressable style={styles.viewer} onPress={() => setOpen(false)} accessibilityLabel="Close picture">
          {source ? <Image source={source} style={styles.full} resizeMode="contain" /> : null}
          <View style={styles.close}>
            <Icon name="close" size={24} color="#fff" />
          </View>
        </Pressable>
      </Modal>
    </>
  );
}

const styles = StyleSheet.create({
  strip: { gap: space.sm, paddingHorizontal: space.md, paddingTop: space.md, alignItems: 'center' },
  menu: {
    position: 'absolute',
    minWidth: 200,
    paddingVertical: space.xs,
    borderRadius: radius.md,
    backgroundColor: m3.surfaceContainerHigh,
    borderWidth: 1,
    borderColor: m3.outlineVariant,
    shadowColor: '#000',
    shadowOpacity: 0.3,
    shadowRadius: 12,
    shadowOffset: { width: 0, height: 4 },
    elevation: 6,
  },
  menuItem: { flexDirection: 'row', alignItems: 'center', gap: space.md, height: 44, paddingHorizontal: space.lg },
  fileChip: {
    flexDirection: 'row',
    alignItems: 'center',
    gap: space.sm,
    height: 52,
    maxWidth: 240,
    paddingHorizontal: space.md,
    borderRadius: radius.md,
    backgroundColor: m3.surfaceContainerHigh,
    borderWidth: 1,
    borderColor: m3.outlineVariant,
  },
  fileText: { flexShrink: 1, minWidth: 0 },
  sentFiles: { flexDirection: 'row', flexWrap: 'wrap', justifyContent: 'flex-end', gap: space.sm, marginBottom: space.sm },
  thumbWrap: { width: 64, height: 64 },
  thumb: { width: 64, height: 64, borderRadius: radius.md, backgroundColor: m3.surfaceContainerHigh },
  remove: {
    position: 'absolute',
    top: -6,
    right: -6,
    width: 22,
    height: 22,
    borderRadius: 11,
    alignItems: 'center',
    justifyContent: 'center',
    backgroundColor: m3.surfaceContainerHighest,
    borderWidth: 1,
    borderColor: m3.outlineVariant,
  },
  picture: {
    width: 240,
    maxWidth: '100%',
    borderRadius: radius.lg,
    overflow: 'hidden',
    backgroundColor: m3.surfaceContainerHigh,
  },
  viewer: { flex: 1, backgroundColor: 'rgba(0,0,0,0.92)', alignItems: 'center', justifyContent: 'center' },
  full: { width: '100%', height: '100%' },
  close: { position: 'absolute', top: 24, right: 24 },
});
