import type { ReactNode } from 'react';
import { KeyboardAvoidingView, Modal, Platform, Pressable, StyleSheet, Text, View } from 'react-native';
import { useSafeAreaInsets } from 'react-native-safe-area-context';

import { useWide } from '../lib/layout';
import { m3, radius, space, type } from './theme';

/**
 * M3 modal: a bottom sheet with a drag handle on phones, a centered dialog
 * on wide (desktop) windows.
 */
export function Sheet({
  visible,
  onClose,
  title,
  children,
}: {
  visible: boolean;
  onClose: () => void;
  title: string;
  children: ReactNode;
}) {
  const insets = useSafeAreaInsets();
  const wide = useWide();
  return (
    <Modal
      visible={visible}
      animationType={wide ? 'fade' : 'slide'}
      transparent
      onRequestClose={onClose}
      statusBarTranslucent
    >
      <KeyboardAvoidingView
        style={[styles.wrap, wide && styles.wrapWide]}
        behavior={Platform.OS === 'ios' ? 'padding' : 'height'}
      >
        <Pressable style={styles.backdrop} onPress={onClose} accessibilityLabel="Close" />
        <View
          style={wide ? styles.dialog : [styles.sheet, { paddingBottom: Math.max(insets.bottom, space.lg) + space.sm }]}
          accessibilityViewIsModal
        >
          {wide ? null : <View style={styles.handle} />}
          <Text style={[type.headlineSmall, styles.title]}>{title}</Text>
          {children}
        </View>
      </KeyboardAvoidingView>
    </Modal>
  );
}

const styles = StyleSheet.create({
  wrap: { flex: 1, justifyContent: 'flex-end' },
  wrapWide: { justifyContent: 'center', alignItems: 'center', padding: space.xl },
  backdrop: { ...StyleSheet.absoluteFill, backgroundColor: m3.scrim },
  sheet: {
    backgroundColor: m3.surfaceContainerLow,
    borderTopLeftRadius: radius.xl,
    borderTopRightRadius: radius.xl,
    paddingHorizontal: space.xl,
  },
  dialog: {
    width: '100%',
    maxWidth: 520,
    backgroundColor: m3.surfaceContainerHigh,
    borderRadius: radius.xl,
    padding: space.xl,
    paddingTop: space.xl,
  },
  handle: {
    alignSelf: 'center',
    width: 32,
    height: 4,
    borderRadius: 2,
    backgroundColor: m3.outline,
    marginVertical: space.lg,
  },
  title: { marginBottom: space.xl },
});
