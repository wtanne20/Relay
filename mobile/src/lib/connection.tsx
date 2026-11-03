import { createContext, useCallback, useContext, useEffect, useMemo, useState, type ReactNode } from 'react';

import { RelayClient } from './api';
import { clearCredentials, loadCredentials, saveCredentials, type Credentials } from './storage';

interface ConnectionValue {
  /** True until saved credentials have been read from secure storage. */
  loading: boolean;
  creds: Credentials | null;
  client: RelayClient | null;
  save: (creds: Credentials) => Promise<void>;
  disconnect: () => Promise<void>;
}

const ConnectionContext = createContext<ConnectionValue | null>(null);

export function ConnectionProvider({ children }: { children: ReactNode }) {
  const [loading, setLoading] = useState(true);
  const [creds, setCreds] = useState<Credentials | null>(null);

  useEffect(() => {
    let alive = true;
    loadCredentials().then((c) => {
      if (!alive) return;
      setCreds(c);
      setLoading(false);
    });
    return () => {
      alive = false;
    };
  }, []);

  const save = useCallback(async (c: Credentials) => {
    await saveCredentials(c);
    setCreds(c);
  }, []);

  const disconnect = useCallback(async () => {
    await clearCredentials();
    setCreds(null);
  }, []);

  const client = useMemo(() => (creds ? new RelayClient(creds) : null), [creds]);
  const value = useMemo(
    () => ({ loading, creds, client, save, disconnect }),
    [loading, creds, client, save, disconnect],
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
