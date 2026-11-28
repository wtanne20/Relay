/**
 * First launch of the desktop app: what is this computer? Either the brain
 * (it runs the model) or a computer linked to a brain elsewhere on the
 * tailnet (it shares its files; the brain does the thinking).
 */
import { router } from 'expo-router';
import { useCallback, useEffect, useState } from 'react';
import { ActivityIndicator, Pressable, ScrollView, StyleSheet, Text, View } from 'react-native';

import { useConnection } from '../lib/connection';
import { desktop, type FoundBrain } from '../lib/desktop';
import { m3, radius, space, type } from './theme';
import { Banner, Button, Field, Icon, Logo, type IconName } from './ui';

export function DesktopSetup() {
  const { mode, reload } = useConnection();
  const [step, setStep] = useState<'choose' | 'link'>('choose');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const brain = async () => {
    setBusy(true);
    setError(null);
    try {
      await desktop.becomeBrain();
      reload();
      router.replace('/');
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const host = mode?.hostname ?? 'this computer';
  return (
    <ScrollView contentContainerStyle={styles.page}>
      <View style={styles.column}>
        <View style={styles.hero}>
          <Logo size={48} />
          <Text style={type.headlineMedium}>Set up Relay on {host}</Text>
          <Text style={[type.bodyLarge, styles.muted]}>
            One computer is the brain: it runs the AI model and keeps your chats. Your other computers link to it, so
            the AI can work with their files while the brain does the thinking.
          </Text>
        </View>
        {step === 'choose' ? (
          <>
            <Choice
              icon="brain"
              title="This computer is the brain"
              body="Runs the model here. Pick this on your most powerful computer, the one with the graphics card."
              onPress={brain}
              busy={busy}
            />
            <Choice
              icon="laptop"
              title="Use another computer as the brain"
              body="This computer shares its files with your brain. You'll need the brain running, Tailscale on both computers, and a code from the brain."
              onPress={() => setStep('link')}
            />
            {error ? <Banner message={error} /> : null}
          </>
        ) : (
          <LinkForm
            home={mode?.mode === 'setup' ? mode.home : ''}
            onBack={() => setStep('choose')}
            onLinked={() => {
              reload();
              router.replace('/');
            }}
          />
        )}
      </View>
    </ScrollView>
  );
}

function Choice({
  icon,
  title,
  body,
  onPress,
  busy,
}: {
  icon: IconName;
  title: string;
  body: string;
  onPress: () => void;
  busy?: boolean;
}) {
  return (
    <Pressable
      onPress={onPress}
      disabled={busy}
      style={({ pressed, hovered }: { pressed: boolean; hovered?: boolean }) => [
        styles.choice,
        (hovered || pressed) && styles.choiceHover,
      ]}
      accessibilityRole="button"
      accessibilityLabel={title}
    >
      <View style={styles.choiceIcon}>
        <Icon name={icon} size={26} color={m3.onPrimaryContainer} />
      </View>
      <View style={styles.grow}>
        <Text style={type.titleMedium}>{title}</Text>
        <Text style={[type.bodyMedium, styles.muted]}>{body}</Text>
      </View>
      {busy ? <ActivityIndicator color={m3.primary} /> : <Icon name="chevron-right" size={22} />}
    </Pressable>
  );
}

function LinkForm({ home, onBack, onLinked }: { home: string; onBack: () => void; onLinked: () => void }) {
  const [found, setFound] = useState<FoundBrain[] | null>(null);
  const [searching, setSearching] = useState(false);
  const [brain, setBrain] = useState('');
  const [manual, setManual] = useState(false);
  const [code, setCode] = useState('');
  const [root, setRoot] = useState(home);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const search = useCallback(async () => {
    setSearching(true);
    try {
      const list = await desktop.findBrains();
      setFound(list);
      if (list.length === 1) setBrain((b) => b || list[0].url);
      if (list.length === 0) setManual(true);
    } catch {
      setFound([]);
      setManual(true);
    } finally {
      setSearching(false);
    }
  }, []);

  useEffect(() => {
    search();
  }, [search]);

  const digits = code.replace(/\D/g, '').slice(0, 6);
  const connect = async () => {
    setBusy(true);
    setError(null);
    try {
      await desktop.linkToBrain(brain.trim(), digits, root.trim() || undefined);
      onLinked();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <View style={styles.form}>
      <Text style={type.titleMedium}>1. Pick your brain</Text>
      {searching ? (
        <View style={styles.searching}>
          <ActivityIndicator size="small" color={m3.primary} />
          <Text style={[type.bodyMedium, styles.muted]}>Looking on your tailnet…</Text>
        </View>
      ) : null}
      {found?.map((b) => (
        <Pressable
          key={b.url}
          onPress={() => {
            setBrain(b.url);
            setManual(false);
          }}
          style={({ pressed }) => [styles.brainRow, brain === b.url && styles.brainOn, pressed && styles.pressed]}
          accessibilityRole="radio"
          accessibilityState={{ selected: brain === b.url }}
        >
          <Icon name="brain" size={20} color={brain === b.url ? m3.onSecondaryContainer : m3.onSurfaceVariant} />
          <View style={styles.grow}>
            <Text style={type.labelLarge}>{b.name}</Text>
            <Text style={[type.bodySmall, styles.muted]}>
              {b.url} · Relay {b.version}
            </Text>
          </View>
          {brain === b.url ? <Icon name="check" size={20} color={m3.primary} /> : null}
        </Pressable>
      ))}
      {found && found.length === 0 && !searching ? (
        <Text style={[type.bodyMedium, styles.muted]}>
          No brain found. Check that Relay is running on your main computer and Tailscale is connected on both, then
          search again, or type its address.
        </Text>
      ) : null}
      <View style={styles.row}>
        <Button title="Search again" variant="text" icon="refresh" onPress={search} disabled={searching} />
        {!manual ? <Button title="Type an address" variant="text" onPress={() => setManual(true)} /> : null}
      </View>
      {manual ? (
        <Field
          label="Brain address"
          icon="lan-connect"
          placeholder="my-desktop or 100.101.102.103"
          hint="Its Tailscale name or address. Relay adds the port."
          value={brain}
          onChangeText={setBrain}
          autoCapitalize="none"
        />
      ) : null}

      <Text style={[type.titleMedium, styles.step]}>2. Enter the code from the brain</Text>
      <Text style={[type.bodyMedium, styles.muted]}>
        On the brain, open Settings, then Computers, then Link a computer.
      </Text>
      <Field
        label="Pairing code"
        icon="numeric"
        placeholder="123 456"
        value={digits.length > 3 ? `${digits.slice(0, 3)} ${digits.slice(3)}` : digits}
        onChangeText={setCode}
        keyboardType="number-pad"
        onSubmitEditing={connect}
      />

      <Text style={[type.titleMedium, styles.step]}>3. Folder the AI may use here</Text>
      <Field
        label="Shared folder"
        icon="folder-outline"
        value={root}
        onChangeText={setRoot}
        hint="The brain can read, change and run things only inside this folder. You pick project folders inside it later."
        autoCapitalize="none"
      />

      {error ? <Banner message={error} /> : null}
      <View style={styles.actions}>
        <Button title="Back" variant="text" onPress={onBack} />
        <Button
          title="Link this computer"
          icon="link-variant"
          onPress={connect}
          loading={busy}
          disabled={!brain.trim() || digits.length !== 6}
        />
      </View>
    </View>
  );
}

const styles = StyleSheet.create({
  page: { flexGrow: 1, padding: space.xl, backgroundColor: m3.surface, alignItems: 'center' },
  column: { width: '100%', maxWidth: 640, gap: space.lg },
  hero: { gap: space.md, alignItems: 'flex-start', marginBottom: space.md },
  muted: { color: m3.onSurfaceVariant },
  grow: { flex: 1, minWidth: 0 },
  choice: {
    flexDirection: 'row',
    alignItems: 'center',
    gap: space.lg,
    padding: space.lg,
    borderRadius: radius.lg,
    backgroundColor: m3.surfaceContainerHigh,
  },
  choiceHover: { backgroundColor: m3.surfaceContainerHighest },
  choiceIcon: {
    width: 48,
    height: 48,
    borderRadius: 24,
    backgroundColor: m3.primaryContainer,
    alignItems: 'center',
    justifyContent: 'center',
  },
  form: { gap: space.sm },
  step: { marginTop: space.lg },
  searching: { flexDirection: 'row', alignItems: 'center', gap: space.sm },
  brainRow: {
    flexDirection: 'row',
    alignItems: 'center',
    gap: space.md,
    padding: space.md,
    borderRadius: radius.md,
    borderWidth: 1,
    borderColor: m3.outlineVariant,
  },
  brainOn: { backgroundColor: m3.secondaryContainer, borderColor: m3.secondaryContainer },
  pressed: { opacity: 0.7 },
  row: { flexDirection: 'row', gap: space.sm },
  actions: { flexDirection: 'row', justifyContent: 'flex-end', gap: space.sm, marginTop: space.md },
});
