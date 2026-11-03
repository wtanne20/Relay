import { useEffect, useState } from 'react';
import { Alert, Platform, StyleSheet, Text, View, type AlertButton } from 'react-native';

import { Sheet } from './Sheet';
import { m3, space, type } from './theme';
import { Button } from './ui';

interface Pending {
  title: string;
  message?: string;
  buttons: AlertButton[];
}

let show: ((p: Pending) => void) | null = null;

/**
 * Drop-in for Alert.alert. Phones get the native alert; the desktop app
 * (react-native-web, where Alert does nothing) gets an M3 dialog.
 */
export function alert(title: string, message?: string, buttons?: AlertButton[]) {
  if (Platform.OS !== 'web') {
    Alert.alert(title, message, buttons);
    return;
  }
  const list = buttons && buttons.length ? buttons : [{ text: 'OK' }];
  if (show) show({ title, message, buttons: list });
  else globalThis.alert?.(`${title}${message ? `\n\n${message}` : ''}`);
}

/** Renders dialogs requested through alert(); mount once near the root. */
export function DialogHost() {
  const [pending, setPending] = useState<Pending | null>(null);
  useEffect(() => {
    show = setPending;
    return () => {
      show = null;
    };
  }, []);
  const close = () => {
    const cancel = pending?.buttons.find((b) => b.style === 'cancel');
    setPending(null);
    cancel?.onPress?.();
  };
  return (
    <Sheet visible={pending !== null} onClose={close} title={pending?.title ?? ''}>
      {pending?.message ? <Text style={[type.bodyLarge, styles.message]}>{pending.message}</Text> : null}
      <View style={styles.actions}>
        {pending?.buttons.map((b, i) => (
          <Button
            key={`${b.text}-${i}`}
            title={b.text ?? 'OK'}
            variant={b.style === 'destructive' ? 'danger' : b.style === 'cancel' ? 'text' : 'filled'}
            onPress={() => {
              setPending(null);
              b.onPress?.();
            }}
          />
        ))}
      </View>
    </Sheet>
  );
}

const styles = StyleSheet.create({
  message: { marginBottom: space.lg, color: m3.onSurfaceVariant },
  actions: { flexDirection: 'row', justifyContent: 'flex-end', gap: space.sm, flexWrap: 'wrap' },
});
