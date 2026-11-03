import { useCallback, useEffect, useRef, useState } from 'react';
import { AppState, Platform } from 'react-native';

import { describeError, type RelayClient } from './api';
import type { Draft, Message, ServerEvent, SessionStatus } from './types';

export type LinkState = 'connecting' | 'live' | 'offline';

type RNWebSocketCtor = new (
  url: string,
  protocols?: string | string[] | null,
  options?: { headers?: Record<string, string> },
) => WebSocket;

const PAGE = 50;
const MAX_BACKOFF_MS = 15000;

function mergeMessages(existing: Message[], incoming: Message[]): Message[] {
  if (incoming.length === 0) return existing;
  const byId = new Map<number, Message>();
  for (const m of existing) byId.set(m.id, m);
  for (const m of incoming) byId.set(m.id, m);
  return Array.from(byId.values()).sort((a, b) => a.id - b.id);
}

/**
 * Keeps a session's history and live stream in sync:
 * loads the latest page over REST, opens the WebSocket, upserts `message`
 * events by id, accumulates deltas into a draft, and reconnects with backoff
 * (immediately when the app returns to the foreground), catching up on
 * anything missed via `?after=`.
 */
export function useSessionStream(client: RelayClient | null, sessionId: string) {
  const [messages, setMessages] = useState<Message[]>([]);
  const [draft, setDraft] = useState<Draft | null>(null);
  const [status, setStatus] = useState<SessionStatus>('idle');
  const [taskError, setTaskError] = useState<string | null>(null);
  const [link, setLink] = useState<LinkState>('connecting');
  const [linkError, setLinkError] = useState<string | null>(null);
  const [hasMore, setHasMore] = useState(false);
  const [loadingOlder, setLoadingOlder] = useState(false);
  const [initialLoaded, setInitialLoaded] = useState(false);

  const messagesRef = useRef<Message[]>([]);
  const wsRef = useRef<WebSocket | null>(null);
  const retryRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const backoffRef = useRef(1000);
  const aliveRef = useRef(true);
  const foregroundRef = useRef(AppState.currentState === 'active');

  const applyMessages = useCallback((incoming: Message[]) => {
    messagesRef.current = mergeMessages(messagesRef.current, incoming);
    setMessages(messagesRef.current);
  }, []);

  const lastId = () => {
    const m = messagesRef.current;
    return m.length ? m[m.length - 1].id : undefined;
  };

  /** Fetch everything after the newest message we have (or the latest page). */
  const catchUp = useCallback(async () => {
    if (!client) return;
    const after = lastId();
    if (after === undefined) {
      const page = await client.messages(sessionId, { limit: PAGE });
      applyMessages(page.messages);
      setHasMore(page.has_more);
      setInitialLoaded(true);
      return;
    }
    let cursor: number | undefined = after;
    // Page forward until caught up.
    for (let i = 0; i < 20 && cursor !== undefined; i++) {
      const page = await client.messages(sessionId, { after: cursor, limit: 200 });
      applyMessages(page.messages);
      cursor = page.has_more ? lastId() : undefined;
    }
    setInitialLoaded(true);
  }, [client, sessionId, applyMessages]);

  const handleEvent = useCallback(
    (ev: ServerEvent) => {
      switch (ev.type) {
        case 'hello':
          setStatus(ev.status);
          setTaskError(ev.status === 'error' ? ev.error : null);
          setDraft(ev.running && ev.draft ? ev.draft : null);
          if (ev.last_message_id !== null && ev.last_message_id !== lastId()) {
            catchUp().catch((e) => setLinkError(describeError(e)));
          }
          break;
        case 'status':
          setStatus(ev.status);
          if (ev.status === 'running') setTaskError(null);
          if (ev.status === 'error') setTaskError(ev.error);
          if (ev.status !== 'running') setDraft(null);
          break;
        case 'text_delta':
          setDraft((d) => ({ content: (d?.content ?? '') + ev.text, reasoning: d?.reasoning ?? '' }));
          break;
        case 'reasoning_delta':
          setDraft((d) => ({ content: d?.content ?? '', reasoning: (d?.reasoning ?? '') + ev.text }));
          break;
        case 'message':
          applyMessages([ev.message]);
          if (ev.message.role === 'assistant') setDraft(null);
          break;
        case 'error':
          setTaskError(ev.message);
          break;
        case 'resync':
          catchUp().catch((e) => setLinkError(describeError(e)));
          break;
        case 'tool_call':
        case 'tool_result':
        case 'final':
          // Tool rows are derived from stored messages, which arrive as
          // `message` events right after these.
          break;
      }
    },
    [applyMessages, catchUp],
  );

  const scheduleReconnect = useCallback(() => {
    if (!aliveRef.current || !foregroundRef.current) return;
    if (retryRef.current) clearTimeout(retryRef.current);
    const delay = backoffRef.current;
    backoffRef.current = Math.min(backoffRef.current * 2, MAX_BACKOFF_MS);
    retryRef.current = setTimeout(() => connectRef.current(), delay);
  }, []);

  const connect = useCallback(() => {
    if (!client || !aliveRef.current) return;
    if (retryRef.current) {
      clearTimeout(retryRef.current);
      retryRef.current = null;
    }
    const old = wsRef.current;
    if (old) {
      old.onclose = null;
      old.onerror = null;
      old.onmessage = null;
      old.close();
    }
    setLink('connecting');
    const Ctor = WebSocket as unknown as RNWebSocketCtor;
    // React Native sends the Authorization header; browsers can't, so the
    // web build (development only) falls back to the ?token= query parameter.
    const ws =
      Platform.OS === 'web'
        ? new Ctor(`${client.wsUrl(sessionId)}?token=${encodeURIComponent(client.creds.token)}`)
        : new Ctor(client.wsUrl(sessionId), undefined, {
            headers: { Authorization: `Bearer ${client.creds.token}` },
          });
    wsRef.current = ws;
    ws.onopen = () => {
      backoffRef.current = 1000;
      setLink('live');
      setLinkError(null);
      // Subscribe first, then fetch history, so nothing falls in the gap.
      catchUp().catch((e) => setLinkError(describeError(e)));
    };
    ws.onmessage = (e: MessageEvent) => {
      if (typeof e.data !== 'string' || e.data === 'pong') return;
      try {
        handleEvent(JSON.parse(e.data) as ServerEvent);
      } catch {
        // Ignore malformed frames.
      }
    };
    ws.onerror = () => {
      setLinkError(`Lost connection to ${client.host}. Retrying…`);
    };
    ws.onclose = () => {
      if (wsRef.current !== ws) return;
      wsRef.current = null;
      setLink('offline');
      scheduleReconnect();
    };
  }, [client, sessionId, catchUp, handleEvent, scheduleReconnect]);

  const connectRef = useRef(connect);
  connectRef.current = connect;

  useEffect(() => {
    aliveRef.current = true;
    messagesRef.current = [];
    setMessages([]);
    setInitialLoaded(false);
    // Load history immediately even before the socket opens (slow links).
    catchUp().catch((e) => setLinkError(describeError(e)));
    connect();
    const sub = AppState.addEventListener('change', (state) => {
      const wasForeground = foregroundRef.current;
      foregroundRef.current = state === 'active';
      if (state === 'active' && !wasForeground) {
        backoffRef.current = 1000;
        connectRef.current();
      } else if (state !== 'active' && retryRef.current) {
        clearTimeout(retryRef.current);
        retryRef.current = null;
      }
    });
    return () => {
      aliveRef.current = false;
      sub.remove();
      if (retryRef.current) clearTimeout(retryRef.current);
      const ws = wsRef.current;
      wsRef.current = null;
      if (ws) {
        ws.onclose = null;
        ws.close();
      }
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [client, sessionId]);

  const loadOlder = useCallback(async () => {
    if (!client || loadingOlder || !hasMore) return;
    const first = messagesRef.current[0]?.id;
    if (first === undefined) return;
    setLoadingOlder(true);
    try {
      const page = await client.messages(sessionId, { before: first, limit: PAGE });
      applyMessages(page.messages);
      setHasMore(page.has_more);
    } catch (e) {
      setLinkError(describeError(e));
    } finally {
      setLoadingOlder(false);
    }
  }, [client, sessionId, hasMore, loadingOlder, applyMessages]);

  /** Optimistically marks the session running; events confirm it. */
  const markRunning = useCallback(() => {
    setStatus('running');
    setTaskError(null);
    setDraft({ content: '', reasoning: '' });
  }, []);

  const reconnect = useCallback(() => {
    backoffRef.current = 1000;
    connectRef.current();
  }, []);

  return {
    messages,
    draft,
    status,
    taskError,
    link,
    linkError,
    hasMore,
    loadingOlder,
    initialLoaded,
    loadOlder,
    applyMessages,
    markRunning,
    reconnect,
    setStatus,
  };
}
