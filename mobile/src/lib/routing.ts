import { useFocusEffect } from 'expo-router';
import { useCallback, useState } from 'react';

import { modelLabel } from '../components/ModelPicker';
import { useConnection } from './connection';
import type { RoutingSettings } from './types';

/**
 * Model routing as it applies right now: the settings when routing is on and
 * can run (a quick model is chosen and Relay runs llama.cpp itself), else
 * null. Re-read whenever the screen gets focus, so it follows changes made on
 * the routing screen.
 */
export function useRouting(): RoutingSettings | null {
  const { client } = useConnection();
  const [routing, setRouting] = useState<RoutingSettings | null>(null);

  useFocusEffect(
    useCallback(() => {
      if (!client) return;
      let alive = true;
      Promise.all([client.settings(), client.model().catch(() => null)])
        .then(([s, m]) => {
          const r = s.settings.routing;
          const on = !!r?.enabled && !!r.quick_model.trim() && !m?.external;
          if (alive) setRouting(on ? r : null);
        })
        .catch(() => undefined);
      return () => {
        alive = false;
      };
    }, [client]),
  );

  return routing;
}

/** The model sheet's first row when it means "let routing choose". */
export function autoRow(routing: RoutingSettings, defaultModel: string | null) {
  const heavy = modelLabel(routing.heavy_model) || modelLabel(defaultModel) || 'the default model';
  return {
    title: 'Auto',
    body: `Quick questions go to ${modelLabel(routing.quick_model)}, everything else to ${heavy}.`,
  };
}
