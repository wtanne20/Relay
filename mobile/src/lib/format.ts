import type { Message, ToolCall } from './types';

export function relativeTime(iso: string, now = Date.now()): string {
  const t = Date.parse(iso);
  if (Number.isNaN(t)) return '';
  const s = Math.max(0, Math.round((now - t) / 1000));
  if (s < 45) return 'just now';
  const m = Math.round(s / 60);
  if (m < 60) return `${m} min ago`;
  const h = Math.round(m / 60);
  if (h < 24) return `${h} h ago`;
  const d = Math.round(h / 24);
  if (d < 7) return `${d} d ago`;
  return new Date(t).toLocaleDateString();
}

export function formatDuration(seconds: number): string {
  const h = Math.floor(seconds / 3600);
  const m = Math.floor((seconds % 3600) / 60);
  if (h > 0) return `${h} h ${m} min`;
  if (m > 0) return `${m} min`;
  return `${seconds} s`;
}

export function parseArgs(call: ToolCall): Record<string, unknown> {
  try {
    const v = JSON.parse(call.arguments || '{}');
    return v && typeof v === 'object' ? (v as Record<string, unknown>) : {};
  } catch {
    return {};
  }
}

function shorten(s: string, max = 60): string {
  const one = s.replace(/\s+/g, ' ').trim();
  return one.length > max ? `${one.slice(0, max - 1)}…` : one;
}

/** One-line summary such as "Ran: cargo build — exit 0". */
export function toolSummary(call: ToolCall, result: Message | undefined, running: boolean): string {
  const a = parseArgs(call);
  const path = typeof a.path === 'string' ? a.path : '.';
  const pending = !result;
  switch (call.name) {
    case 'run_command': {
      const cmd = shorten(typeof a.command === 'string' ? a.command : '');
      if (pending) return running ? `Running: ${cmd}…` : `Ran: ${cmd} — no result`;
      const first = result.content.split('\n', 1)[0] ?? '';
      const exit = first.match(/^exit_code: (-?\d+|unknown)/);
      if (exit) return `Ran: ${cmd} — exit ${exit[1]}`;
      if (first.startsWith('timed out')) return `Ran: ${cmd} — timed out`;
      if (first.startsWith('cancelled')) return `Ran: ${cmd} — cancelled`;
      if (first.startsWith('blocked')) return `Blocked: ${cmd}`;
      return `Ran: ${cmd} — failed`;
    }
    case 'read_file':
      return `${pending ? 'Reading' : 'Read'} ${shorten(path)}${result?.is_error ? ' — failed' : ''}`;
    case 'write_file':
      return `${pending ? 'Writing' : 'Wrote'} ${shorten(path)}${result?.is_error ? ' — failed' : ''}`;
    case 'edit_file':
      return `${pending ? 'Editing' : 'Edited'} ${shorten(path)}${result?.is_error ? ' — failed' : ''}`;
    case 'web_search': {
      const q = shorten(typeof a.query === 'string' ? a.query : '');
      return `${pending ? 'Searching the web for' : 'Searched the web for'} “${q}”${result?.is_error ? ' — failed' : ''}`;
    }
    case 'fetch_url': {
      const url = typeof a.url === 'string' ? a.url : '';
      const host = url.replace(/^https?:\/\//i, '').split(/[/?#]/)[0] || url;
      return `${pending ? 'Reading' : 'Read'} ${shorten(host)}${result?.is_error ? ' — failed' : ''}`;
    }
    case 'search_documents': {
      const q = shorten(typeof a.query === 'string' ? a.query : '');
      return `${pending ? 'Searching your documents for' : 'Searched your documents for'} “${q}”${result?.is_error ? ' — failed' : ''}`;
    }
    case 'read_document':
      return `${pending ? 'Reading' : 'Read'} ${shorten(basename(path))}${result?.is_error ? ' — failed' : ''}`;
    case 'list_dir':
      return `${pending ? 'Listing' : 'Listed'} ${shorten(path)}${result?.is_error ? ' — failed' : ''}`;
    default:
      return `${call.name}${result?.is_error ? ' — failed' : ''}`;
  }
}

export function toolDetails(call: ToolCall): string {
  const a = parseArgs(call);
  if (call.name === 'run_command' && typeof a.command === 'string') return a.command;
  if (call.name === 'edit_file') {
    return `${String(a.path ?? '')}\n--- old ---\n${String(a.old_text ?? '')}\n--- new ---\n${String(a.new_text ?? '')}`;
  }
  if (call.name === 'write_file') {
    return `${String(a.path ?? '')}\n---\n${String(a.content ?? '')}`;
  }
  try {
    return JSON.stringify(JSON.parse(call.arguments || '{}'), null, 2);
  } catch {
    return call.arguments;
  }
}

/** Last path segment of a Windows or POSIX path. */
export function basename(path: string): string {
  const parts = path.split(/[\\/]+/).filter(Boolean);
  return parts.length ? parts[parts.length - 1] : path;
}

export function formatBytes(n: number): string {
  if (n >= 1024 ** 4) return `${(n / 1024 ** 4).toFixed(2)} TB`;
  if (n >= 1024 ** 3) return `${(n / 1024 ** 3).toFixed(1)} GB`;
  if (n >= 1024 ** 2) return `${Math.round(n / 1024 ** 2)} MB`;
  return `${Math.round(n / 1024)} KB`;
}
