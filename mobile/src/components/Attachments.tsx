import * as ImagePicker from 'expo-image-picker';
import { useCallback, useEffect, useRef, useState } from 'react';
import { Image, Modal, Platform, Pressable, ScrollView, StyleSheet, View } from 'react-native';

import type { RelayClient } from '../lib/api';
import { useClient } from '../lib/connection';
import { m3, radius, space } from './theme';
import { Icon, IconButton } from './ui';

/** Matches the server's limit per prompt. */
export const MAX_PICTURES = 8;

export type Attachment = { id: string; dataUrl: string };

let counter = 0;

function readFile(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const r = new FileReader();
    r.onload = () => resolve(String(r.result));
    r.onerror = () => reject(r.error);
    r.readAsDataURL(file);
  });
}

function imageFiles(list: FileList | null | undefined): File[] {
  return Array.from(list ?? []).filter((f) => f.type.startsWith('image/'));
}

/**
 * Pictures waiting to be sent with the next prompt. On the web and desktop,
 * pictures can also be pasted or dropped anywhere on the page.
 */
export function useAttachments() {
  const [items, setItems] = useState<Attachment[]>([]);
  const [error, setError] = useState<string | null>(null);

  const add = useCallback((urls: string[]) => {
    if (urls.length === 0) return;
    setItems((cur) => {
      const next = [...cur, ...urls.map((dataUrl) => ({ id: `a${++counter}`, dataUrl }))];
      if (next.length > MAX_PICTURES) setError(`You can attach up to ${MAX_PICTURES} pictures at a time.`);
      return next.slice(0, MAX_PICTURES);
    });
  }, []);

  const addFiles = useCallback(
    (files: File[]) => {
      Promise.all(files.map(readFile))
        .then(add)
        .catch(() => setError("Couldn't read that picture."));
    },
    [add],
  );

  useEffect(() => {
    if (Platform.OS !== 'web' || typeof document === 'undefined') return;
    const onPaste = (e: ClipboardEvent) => {
      const files = imageFiles(e.clipboardData?.files);
      if (files.length === 0) return;
      e.preventDefault();
      addFiles(files);
    };
    const hasFiles = (e: DragEvent) => Array.from(e.dataTransfer?.types ?? []).includes('Files');
    const onDragOver = (e: DragEvent) => {
      if (hasFiles(e)) e.preventDefault();
    };
    const onDrop = (e: DragEvent) => {
      if (!hasFiles(e)) return;
      e.preventDefault();
      addFiles(imageFiles(e.dataTransfer?.files));
    };
    document.addEventListener('paste', onPaste);
    document.addEventListener('dragover', onDragOver);
    document.addEventListener('drop', onDrop);
    return () => {
      document.removeEventListener('paste', onPaste);
      document.removeEventListener('dragover', onDragOver);
      document.removeEventListener('drop', onDrop);
    };
  }, [addFiles]);

  const pick = useCallback(
    async (source: 'library' | 'camera') => {
      setError(null);
      if (Platform.OS === 'web') {
        const input = document.createElement('input');
        input.type = 'file';
        input.accept = 'image/*';
        input.multiple = true;
        input.onchange = () => addFiles(imageFiles(input.files));
        input.click();
        return;
      }
      try {
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
            .map((a) => `data:${a.mimeType ?? 'image/jpeg'};base64,${a.base64}`),
        );
      } catch {
        setError("Couldn't open your pictures.");
      }
    },
    [add, addFiles],
  );

  const remove = useCallback((id: string) => setItems((cur) => cur.filter((a) => a.id !== id)), []);
  const clear = useCallback(() => {
    setItems([]);
    setError(null);
  }, []);

  return { items, error, setError, pick, remove, clear };
}

/** Attach buttons for the composer: one on desktop, gallery and camera on a phone. */
export function AttachButtons({
  onPick,
  disabled,
}: {
  onPick: (source: 'library' | 'camera') => void;
  disabled?: boolean;
}) {
  if (Platform.OS === 'web') {
    return <IconButton icon="image-plus" label="Attach pictures" onPress={() => onPick('library')} disabled={disabled} size={22} />;
  }
  return (
    <>
      <IconButton icon="image-outline" label="Attach pictures" onPress={() => onPick('library')} disabled={disabled} size={22} />
      <IconButton icon="camera-outline" label="Take a photo" onPress={() => onPick('camera')} disabled={disabled} size={22} />
    </>
  );
}

/** Thumbnails of the pictures about to be sent, each with a remove button. */
export function AttachmentStrip({ items, onRemove }: { items: Attachment[]; onRemove: (id: string) => void }) {
  if (items.length === 0) return null;
  return (
    <ScrollView horizontal showsHorizontalScrollIndicator={false} contentContainerStyle={styles.strip}>
      {items.map((a) => (
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
      ))}
    </ScrollView>
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
  strip: { gap: space.sm, paddingHorizontal: space.md, paddingTop: space.md },
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
