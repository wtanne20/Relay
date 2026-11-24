import { router } from 'expo-router';
import { useState } from 'react';
import {
  ActivityIndicator,
  Image,
  Modal,
  Platform,
  Pressable,
  ScrollView,
  StyleSheet,
  Text,
  View,
  type ImageStyle,
  type StyleProp,
} from 'react-native';

import { useClient } from '../lib/connection';
import type { ImageModel } from '../lib/images';
import { useAuthedUri } from './Attachments';
import { Sheet } from './Sheet';
import { m3, radius, space, type } from './theme';
import { Button, Icon, type IconName } from './ui';

/** A stored picture (from /attachments), loaded with the token. */
export function StoredImage({
  name,
  style,
  resizeMode = 'cover',
}: {
  name: string;
  style?: StyleProp<ImageStyle>;
  resizeMode?: 'cover' | 'contain';
}) {
  const client = useClient();
  const uri = useAuthedUri(client, name);
  if (!uri || !client) return <View style={[style as object, styles.placeholder]} />;
  const source =
    Platform.OS === 'web' ? { uri } : { uri, headers: { Authorization: `Bearer ${client.creds.token}` } };
  return <Image source={source} style={style} resizeMode={resizeMode} accessibilityIgnoresInvertColors />;
}

/** Full-screen picture with actions along the bottom. */
export function PictureViewer({
  name,
  onClose,
  actions,
}: {
  name: string | null;
  onClose: () => void;
  actions?: { icon: IconName; label: string; onPress: () => void }[];
}) {
  const client = useClient();
  const uri = useAuthedUri(client, name ?? '');
  return (
    <Modal visible={!!name} transparent animationType="fade" onRequestClose={onClose}>
      <View style={styles.viewer}>
        <Pressable style={StyleSheet.absoluteFill} onPress={onClose} accessibilityLabel="Close picture" />
        {name ? <StoredImage name={name} style={styles.full} resizeMode="contain" /> : null}
        <Pressable onPress={onClose} style={styles.close} accessibilityRole="button" accessibilityLabel="Close">
          <Icon name="close" size={26} color="#fff" />
        </Pressable>
        <View style={styles.viewerActions}>
          {Platform.OS === 'web' && uri ? (
            <ViewerAction
              icon="download"
              label="Save"
              onPress={() => {
                const a = document.createElement('a');
                a.href = uri;
                a.download = name ?? 'picture.png';
                a.click();
              }}
            />
          ) : null}
          {actions?.map((a) => (
            <ViewerAction
              key={a.label}
              {...a}
              onPress={() => {
                onClose();
                a.onPress();
              }}
            />
          ))}
        </View>
      </View>
    </Modal>
  );
}

function ViewerAction({ icon, label, onPress }: { icon: IconName; label: string; onPress: () => void }) {
  return (
    <Pressable
      onPress={onPress}
      style={({ pressed }) => [styles.viewerAction, pressed && { opacity: 0.8 }]}
      accessibilityRole="button"
      accessibilityLabel={label}
    >
      <Icon name={icon} size={20} color="#fff" />
      <Text style={[type.labelLarge, { color: '#fff' }]}>{label}</Text>
    </Pressable>
  );
}

/** Chooses one of the saved image models, or goes to set one up. */
export function ImageModelSheet({
  visible,
  models,
  value,
  onPick,
  onClose,
  title = 'Image model',
}: {
  visible: boolean;
  models: ImageModel[];
  value: string | null | undefined;
  onPick: (id: string) => void;
  onClose: () => void;
  title?: string;
}) {
  const [busy, setBusy] = useState<string | null>(null);
  return (
    <Sheet visible={visible} onClose={onClose} title={title}>
      <ScrollView style={{ maxHeight: 420 }}>
        {models.length === 0 ? (
          <Text style={[type.bodyMedium, styles.hint]}>
            No image models yet. Put model files in Relay&apos;s image model folder (or point Relay at your ComfyUI
            models folder), then set one up.
          </Text>
        ) : null}
        {models.map((m) => {
          const on = m.id === value;
          return (
            <Pressable
              key={m.id}
              onPress={async () => {
                setBusy(m.id);
                try {
                  await onPick(m.id);
                } finally {
                  setBusy(null);
                }
              }}
              style={({ pressed }) => [styles.option, on && styles.optionOn, pressed && styles.optionPressed]}
              accessibilityRole="radio"
              accessibilityState={{ checked: on }}
            >
              <Icon name="image-filter-hdr" size={22} color={on ? m3.onSecondaryContainer : m3.onSurfaceVariant} />
              <View style={styles.grow}>
                <Text style={[type.bodyLarge, on && { color: m3.onSecondaryContainer }]} numberOfLines={1}>
                  {m.name}
                </Text>
                <Text style={[type.bodySmall, styles.sub]} numberOfLines={1}>
                  {m.recipe}
                </Text>
              </View>
              {busy === m.id ? <ActivityIndicator color={m3.primary} /> : on ? <Icon name="check" color={m3.primary} /> : null}
            </Pressable>
          );
        })}
      </ScrollView>
      <View style={styles.sheetActions}>
        <Button
          title="Image models"
          icon="cog-outline"
          variant="text"
          onPress={() => {
            onClose();
            router.push('/image-models');
          }}
        />
        <Button title="Done" variant="tonal" onPress={onClose} />
      </View>
    </Sheet>
  );
}

/** The image model button above or inside the message box. */
export function ImageModelChip({ label, onPress }: { label: string; onPress: () => void }) {
  return (
    <Pressable
      onPress={onPress}
      style={({ pressed, hovered }: { pressed: boolean; hovered?: boolean }) => [
        styles.chip,
        (pressed || hovered) && styles.chipHover,
      ]}
      accessibilityRole="button"
      accessibilityLabel={`Image model: ${label}. Change`}
    >
      <Icon name="image-filter-hdr" size={16} color={m3.onSurfaceVariant} />
      <Text style={[type.labelLarge, styles.chipText]} numberOfLines={1}>
        {label}
      </Text>
      <Icon name="chevron-down" size={16} color={m3.onSurfaceVariant} />
    </Pressable>
  );
}

const styles = StyleSheet.create({
  grow: { flex: 1, minWidth: 0 },
  placeholder: { backgroundColor: m3.surfaceContainerHigh },
  viewer: { flex: 1, backgroundColor: 'rgba(0,0,0,0.94)', alignItems: 'center', justifyContent: 'center' },
  full: { width: '100%', height: '82%' },
  close: { position: 'absolute', top: 28, right: 20, padding: space.sm },
  viewerActions: {
    position: 'absolute',
    bottom: 32,
    flexDirection: 'row',
    flexWrap: 'wrap',
    justifyContent: 'center',
    gap: space.sm,
    paddingHorizontal: space.lg,
  },
  viewerAction: {
    flexDirection: 'row',
    alignItems: 'center',
    gap: space.sm,
    height: 44,
    paddingHorizontal: space.lg,
    borderRadius: 22,
    backgroundColor: 'rgba(255,255,255,0.14)',
  },
  hint: { color: m3.onSurfaceVariant, marginBottom: space.md, lineHeight: 21 },
  option: {
    flexDirection: 'row',
    alignItems: 'center',
    gap: space.md,
    minHeight: 60,
    paddingHorizontal: space.md,
    borderRadius: radius.md,
  },
  optionOn: { backgroundColor: m3.secondaryContainer },
  optionPressed: { backgroundColor: m3.pressed },
  sub: { color: m3.onSurfaceVariant },
  sheetActions: { flexDirection: 'row', justifyContent: 'space-between', marginTop: space.md },
  chip: {
    flexDirection: 'row',
    alignItems: 'center',
    gap: 6,
    height: 36,
    maxWidth: 220,
    paddingHorizontal: space.md,
    borderRadius: 18,
  },
  chipHover: { backgroundColor: m3.hover },
  chipText: { color: m3.onSurfaceVariant, flexShrink: 1 },
});
