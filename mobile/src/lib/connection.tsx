import { createContext, useCallback, useContext, useEffect, useMemo, useState, type ReactNode } from 'react';

import { RelayClient } from './api';
import { desktop, isDesktop } from './desktop';
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
}

const ConnectionContext = createContext<ConnectionValue | null>(null);

export function ConnectionProvider({ children }: { children: ReactNode }) {
  const [loading, setLoading] = useState(true);
  const [creds, setCreds] = useState<Credentials | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let alive = true;
    // The desktop app talks to the server on this PC; the phone uses the
    // credentials saved when it was paired.
    const load: Promise<Credentials | null> = isDesktop
      ? desktop.localConnection().then(
          (c) => ({ url: c.url, token: c.token }),
          (e: unknown) => {
            setError(String(e));
            return null;
          },
        )
      : loadCredentials();
    load.then((c) => {
      if (!alive) return;
      setCreds(c);
      setLoading(false);
    });
    return () => {
      alive = false;
    };
  }, []);

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
    () => ({ loading, creds, error, client, save, disconnect }),
    [loading, creds, error, client, save, disconnect],
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
