import { createContext, useCallback, useContext, useEffect, useMemo, useState, type ReactNode } from 'react';

import { RelayClient } from './api';
import { desktop, isDesktop, type AppMode } from './desktop';
import { clearCredentials, loadCredentials, saveCredentials, type Credentials } from './storage';

interface ConnectionValue {
  /** True until saved credentials have been read from secure storage. */
  loading: boolean;
  creds: Credentials | null;
  /** Set when the desktop app could not read the local server's config. */
  error: string | null;
  client: RelayClient | null;
  save: (creds: Credentials) => Promise<void>;
  disconnect: () => Promise<void>;
  /** Desktop app only: what this computer is (brain, linked, or not set up). */
  mode: AppMode | null;
  /** Desktop app only: re-reads the mode and connection (after setup). */
  reload: () => void;
}

const ConnectionContext = createContext<ConnectionValue | null>(null);

export function ConnectionProvider({ children }: { children: ReactNode }) {
  const [loading, setLoading] = useState(true);
  const [creds, setCreds] = useState<Credentials | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [mode, setMode] = useState<AppMode | null>(null);
  const [generation, setGeneration] = useState(0);
  const reload = useCallback(() => setGeneration((g) => g + 1), []);

  useEffect(() => {
    let alive = true;
    setLoading(true);
    // The desktop app talks to its brain (this computer, or the one it is
    // linked to); the phone uses the credentials saved when it was paired.
    const fromDesktop = async (): Promise<Credentials | null> => {
      const m = await desktop.appMode().catch(() => null);
      if (alive) setMode(m);
      if (m?.mode === 'setup') return null;
      return desktop.localConnection().then(
        (c) => {
          setError(null);
          return { url: c.url, token: c.token };
        },
        (e: unknown) => {
          setError(String(e));
          return null;
        },
      );
    };
    const load: Promise<Credentials | null> = isDesktop ? fromDesktop() : loadCredentials();
    load.then((c) => {
      if (!alive) return;
      setCreds(c);
      setLoading(false);
    });
    return () => {
      alive = false;
    };
  }, [generation]);

  // A linked computer's link state changes on its own; keep it fresh.
  useEffect(() => {
    if (!isDesktop || mode?.mode !== 'device') return;
    const t = setInterval(() => {
      desktop
        .appMode()
        .then((m) => setMode((old) => (JSON.stringify(old) === JSON.stringify(m) ? old : m)))
        .catch(() => undefined);
    }, 5000);
    return () => clearInterval(t);
  }, [mode?.mode]);

  const save = useCallback(async (c: Credentials) => {
    if (!isDesktop) await saveCredentials(c);
    setCreds(c);
  }, []);

  const disconnect = useCallback(async () => {
    if (isDesktop) return;
    await clearCredentials();
    setCreds(null);
  }, []);

  const client = useMemo(() => (creds ? new RelayClient(creds) : null), [creds]);
  const value = useMemo(
    () => ({ loading, creds, error, client, save, disconnect, mode, reload }),
    [loading, creds, error, client, save, disconnect, mode, reload],
  );
  return <ConnectionContext.Provider value={value}>{children}</ConnectionContext.Provider>;
}

export function useConnection(): ConnectionValue {
  const v = useContext(ConnectionContext);
  if (!v) throw new Error('useConnection must be used inside ConnectionProvider');
  return v;
}

/** The client for screens that are only reachable once connected. */
export function useClient(): RelayClient | null {
  return useConnection().client;
}
