import { CameraView, useCameraPermissions, type BarcodeScanningResult } from 'expo-camera';
import { router } from 'expo-router';
import { useRef, useState } from 'react';
import { ActivityIndicator, Linking, StyleSheet, Text, View } from 'react-native';

import { colors, radius, space } from '../components/theme';
import { Banner, Button } from '../components/ui';
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
        <ActivityIndicator color={colors.accent} />
      </View>
    );
  }

  if (!permission.granted) {
    return (
      <View style={styles.center}>
        <Text style={styles.title}>Camera access</Text>
        <Text style={styles.body}>Relay needs the camera only to read the pairing QR code on your PC.</Text>
        {permission.canAskAgain ? (
          <Button title="Allow camera" onPress={requestPermission} style={styles.button} />
        ) : (
          <Button title="Open settings" onPress={() => Linking.openSettings()} style={styles.button} />
        )}
        <Button title="Enter the address instead" variant="ghost" onPress={() => router.back()} />
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
        <View style={styles.frame} />
        <Text style={styles.hint}>Point the camera at the QR code in the Relay window on your PC.</Text>
        {error ? <Banner message={error} /> : null}
      </View>
    </View>
  );
}

const styles = StyleSheet.create({
  flex: { flex: 1, backgroundColor: '#000' },
  center: { flex: 1, backgroundColor: colors.bg, justifyContent: 'center', padding: space.xl, gap: space.md },
  title: { color: colors.text, fontSize: 22, fontWeight: '700' },
  body: { color: colors.textMuted, fontSize: 16, lineHeight: 22 },
  button: { marginTop: space.md },
  overlay: { flex: 1, justifyContent: 'center', alignItems: 'center', padding: space.xl, gap: space.xl },
  frame: { width: 260, height: 260, borderRadius: radius.lg, borderWidth: 3, borderColor: colors.accent },
  hint: {
    color: '#fff',
    fontSize: 16,
    textAlign: 'center',
    backgroundColor: 'rgba(0,0,0,0.6)',
    padding: space.md,
    borderRadius: radius.md,
  },
});
