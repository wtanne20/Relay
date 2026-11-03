import { CameraView, useCameraPermissions, type BarcodeScanningResult } from 'expo-camera';
import { router } from 'expo-router';
import { useRef, useState } from 'react';
import { ActivityIndicator, Linking, StyleSheet, Text, View } from 'react-native';

import { m3, radius, space, type } from '../components/theme';
import { Banner, Button, Icon } from '../components/ui';
import { parsePairingLink } from '../lib/pairing';

/** Scans the pairing QR code shown by Relay on the PC. */
export default function ScanScreen() {
  const [permission, requestPermission] = useCameraPermissions();
  const [error, setError] = useState<string | null>(null);
  const handled = useRef(false);

  const onScanned = (result: BarcodeScanningResult) => {
    if (handled.current) return;
    const creds = parsePairingLink(result.data);
    if (!creds) {
      setError("That QR code isn't a Relay pairing code. Open Relay → Pair phone on your PC.");
      return;
    }
    handled.current = true;
    router.replace({ pathname: '/pair', params: { url: creds.url, token: creds.token } });
  };

  if (!permission) {
    return (
      <View style={styles.center}>
        <ActivityIndicator color={m3.primary} />
      </View>
    );
  }

  if (!permission.granted) {
    return (
      <View style={styles.center}>
        <View style={styles.badge}>
          <Icon name="camera-outline" size={36} color={m3.onPrimaryContainer} />
        </View>
        <Text style={[type.headlineMedium, styles.text]}>Camera access</Text>
        <Text style={[type.bodyLarge, styles.body]}>
          Relay needs the camera only to read the pairing QR code on your PC.
        </Text>
        {permission.canAskAgain ? (
          <Button title="Allow camera" icon="camera" onPress={requestPermission} style={styles.button} />
        ) : (
          <Button
            title="Open settings"
            icon="cog-outline"
            onPress={() => Linking.openSettings()}
            style={styles.button}
          />
        )}
        <Button title="Enter the address instead" variant="text" onPress={() => router.back()} style={styles.button} />
      </View>
    );
  }

  return (
    <View style={styles.flex}>
      <CameraView
        style={StyleSheet.absoluteFill}
        facing="back"
        barcodeScannerSettings={{ barcodeTypes: ['qr'] }}
        onBarcodeScanned={onScanned}
      />
      <View style={styles.overlay} pointerEvents="box-none">
        <View style={styles.frame}>
          <View style={[styles.corner, styles.tl]} />
          <View style={[styles.corner, styles.tr]} />
          <View style={[styles.corner, styles.bl]} />
          <View style={[styles.corner, styles.br]} />
        </View>
        <Text style={[type.bodyLarge, styles.hint]}>
          Point the camera at the QR code in the Relay window on your PC.
        </Text>
        {error ? <Banner message={error} /> : null}
      </View>
    </View>
  );
}

const C = 36;
const W = 4;

const styles = StyleSheet.create({
  flex: { flex: 1, backgroundColor: '#000' },
  center: {
    flex: 1,
    backgroundColor: m3.surface,
    justifyContent: 'center',
    alignItems: 'center',
    padding: space.xl,
    gap: space.md,
  },
  badge: {
    width: 88,
    height: 88,
    borderRadius: radius.xl,
    backgroundColor: m3.primaryContainer,
    alignItems: 'center',
    justifyContent: 'center',
    marginBottom: space.sm,
  },
  text: { textAlign: 'center' },
  body: { color: m3.onSurfaceVariant, textAlign: 'center', maxWidth: 360 },
  button: { alignSelf: 'stretch' },
  overlay: { flex: 1, justifyContent: 'center', alignItems: 'center', padding: space.xl, gap: space.xl },
  frame: { width: 260, height: 260 },
  corner: { position: 'absolute', width: C, height: C, borderColor: m3.primary },
  tl: { top: 0, left: 0, borderTopWidth: W, borderLeftWidth: W, borderTopLeftRadius: radius.xl },
  tr: { top: 0, right: 0, borderTopWidth: W, borderRightWidth: W, borderTopRightRadius: radius.xl },
  bl: { bottom: 0, left: 0, borderBottomWidth: W, borderLeftWidth: W, borderBottomLeftRadius: radius.xl },
  br: { bottom: 0, right: 0, borderBottomWidth: W, borderRightWidth: W, borderBottomRightRadius: radius.xl },
  hint: {
    color: m3.onSurface,
    textAlign: 'center',
    backgroundColor: 'rgba(22,20,19,0.85)',
    paddingVertical: space.md,
    paddingHorizontal: space.lg,
    borderRadius: radius.xl,
    overflow: 'hidden',
  },
});
