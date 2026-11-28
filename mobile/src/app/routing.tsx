import { useFocusEffect } from 'expo-router';
import { useCallback, useState } from 'react';
import { ActivityIndicator, ScrollView, StyleSheet, Text, View } from 'react-native';

import { ModelSheet, modelLabel } from '../components/ModelPicker';
import { Group, NavRow, SectionTitle, Segmented, ToggleRow } from '../components/SettingsControls';
import { m3, space, type } from '../components/theme';
import { Banner } from '../components/ui';
import { describeError } from '../lib/api';
import { useConnection } from '../lib/connection';
import { usePageStyle } from '../lib/layout';
import type { RoutingSettings } from '../lib/types';

const LENGTHS = ['20', '40', '60', '100'] as const;

/**
 * Model routing: quick questions go to a small, fast model and real work to
 * a big one. Every change saves right away.
 */
export default function RoutingScreen() {
  const page = usePageStyle();
  const { client } = useConnection();
  const [routing, setRouting] = useState<RoutingSettings | null>(null);
  const [defaultModel, setDefaultModel] = useState('');
  const [error, setError] = useState<string | null>(null);
  const [picking, setPicking] = useState<'quick' | 'heavy' | null>(null);

  const load = useCallback(async () => {
    if (!client) return;
    try {
      const [s, m] = await Promise.all([client.settings(), client.model().catch(() => null)]);
      setRouting(s.settings.routing);
      setDefaultModel(m?.default_model || s.settings.llama.model_path);
      setError(null);
    } catch (e) {
      setError(describeError(e));
    }
  }, [client]);

  useFocusEffect(
    useCallback(() => {
      load();
    }, [load]),
  );

  if (!client) return null;
  if (!routing) {
    return (
      <View style={styles.center}>
        {error ? <Banner message={error} actionLabel="Retry" onAction={load} /> : <ActivityIndicator color={m3.primary} />}
      </View>
    );
  }

  const save = async (patch: Partial<RoutingSettings>) => {
    const before = routing;
    setRouting({ ...routing, ...patch });
    try {
      const r = await client.patchSettings({ routing: patch });
      setRouting(r.settings.routing);
      setError(null);
    } catch (e) {
      setRouting(before);
      setError(describeError(e));
      throw e;
    }
  };
  const quiet = (patch: Partial<RoutingSettings>) => save(patch).catch(() => {});

  const length = (LENGTHS as readonly string[]).includes(String(routing.quick_max_words))
    ? (String(routing.quick_max_words) as (typeof LENGTHS)[number])
    : '60';
  const heavyLabel = modelLabel(routing.heavy_model) || `Default model (${modelLabel(defaultModel) || 'not set'})`;

  return (
    <ScrollView style={styles.flex} contentContainerStyle={[styles.content, page]}>
      <Text style={[type.bodyMedium, styles.intro]}>
        Send quick questions to a small, fast model and everything else to a big one. Only chats and projects that
        don&apos;t pick their own model are routed. Relay loads one model at a time, so switching takes a moment; if
        the big model is already loaded, quick questions use it instead of waiting.
      </Text>
      {error ? <Banner message={error} /> : null}

      <Group>
        <ToggleRow
          icon="call-split"
          label="Route by question"
          hint={routing.quick_model ? undefined : 'Choose a quick model first'}
          value={routing.enabled}
          disabled={!routing.quick_model}
          onChange={(enabled) => quiet({ enabled })}
        />
      </Group>

      <SectionTitle title="Models" />
      <Group>
        <NavRow
          icon="lightning-bolt-outline"
          label="Quick model"
          value={modelLabel(routing.quick_model) || 'Not chosen'}
          onPress={() => setPicking('quick')}
        />
        <NavRow icon="weight" label="Big model" value={heavyLabel} onPress={() => setPicking('heavy')} />
      </Group>

      <SectionTitle title="What counts as quick" />
      <Group>
        <Segmented
          icon="text-short"
          label="Longest quick question"
          hint="In words. Longer prompts go to the big model."
          value={length}
          options={LENGTHS.map((v) => ({ value: v, label: v }))}
          onChange={(v) => quiet({ quick_max_words: Number(v) })}
        />
      </Group>
      <Text style={[type.bodySmall, styles.note]}>
        The big model also takes project work (it uses tools), anything with pictures or files, code, and requests
        like “write”, “fix”, “plan” or “summarize”. Once a chat needs the big model, it stays on it. Each reply shows
        which model answered.
      </Text>

      <ModelSheet
        visible={picking !== null}
        onClose={() => setPicking(null)}
        value={picking === 'quick' ? routing.quick_model : routing.heavy_model}
        defaultTitle={picking === 'quick' ? 'None' : 'Default model'}
        defaultBody={
          picking === 'quick'
            ? 'Turns routing off.'
            : `The model chosen in Settings (${modelLabel(defaultModel) || 'not set'}).`
        }
        onPick={(path) =>
          picking === 'quick'
            ? save(
                path
                  ? { quick_model: path, ...(routing.quick_model ? {} : { enabled: true }) }
                  : { quick_model: '', enabled: false },
              )
            : save({ heavy_model: path })
        }
      />
    </ScrollView>
  );
}

const styles = StyleSheet.create({
  flex: { flex: 1, backgroundColor: m3.surface },
  content: { padding: space.lg, paddingBottom: 64 },
  center: { flex: 1, alignItems: 'center', justifyContent: 'center', padding: space.lg, backgroundColor: m3.surface },
  intro: { color: m3.onSurfaceVariant, marginBottom: space.lg, lineHeight: 20 },
  note: { color: m3.onSurfaceVariant, marginTop: space.sm, paddingHorizontal: space.sm, lineHeight: 18 },
});
