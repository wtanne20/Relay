/** Tiny app-wide signal so views can refresh after another view changes data. */
type Topic = 'projects' | 'chats';
const listeners = new Map<Topic, Set<() => void>>();

export function emit(topic: Topic) {
  listeners.get(topic)?.forEach((fn) => fn());
}

/** Subscribe; returns the unsubscribe function (for useEffect cleanup). */
export function on(topic: Topic, fn: () => void): () => void {
  if (!listeners.has(topic)) listeners.set(topic, new Set());
  listeners.get(topic)?.add(fn);
  return () => {
    listeners.get(topic)?.delete(fn);
  };
}
