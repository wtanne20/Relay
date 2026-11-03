import { normalizeServerUrl } from './api';
import type { Credentials } from './storage';

/**
 * Parses the pairing link shown as a QR code by the PC:
 * relay://pair?url=http%3A%2F%2F100.x.y.z%3A8765&token=<hex>
 * Returns null for anything else.
 */
export function parsePairingLink(data: string): Credentials | null {
  const text = data.trim();
  const m = text.match(/^relay:\/\/+pair\/?\?(.*)$/i);
  if (!m) return null;
  const params: Record<string, string> = {};
  for (const part of m[1].split('&')) {
    const eq = part.indexOf('=');
    if (eq < 0) continue;
    try {
      params[decodeURIComponent(part.slice(0, eq))] = decodeURIComponent(part.slice(eq + 1).replace(/\+/g, ' '));
    } catch {
      return null;
    }
  }
  return credentialsFrom(params.url, params.token);
}

/** Validates a url/token pair (from a QR code or a deep link). */
export function credentialsFrom(url: unknown, token: unknown): Credentials | null {
  if (typeof url !== 'string' || typeof token !== 'string') return null;
  const u = normalizeServerUrl(url);
  const t = token.trim();
  if (!/^https?:\/\/[^/\s]+$/i.test(u) || !/^[A-Za-z0-9._~-]{16,256}$/.test(t)) return null;
  return { url: u, token: t };
}
