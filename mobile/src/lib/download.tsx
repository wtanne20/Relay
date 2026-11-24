/**
 * Downloads a file from the Relay server and opens it with the system
 * handler (install an APK, open a zip, etc.). Works on Android and the
 * desktop WebView.
 */
import * as FileSystem from 'expo-file-system/legacy';
import * as IntentLauncher from 'expo-intent-launcher';
import { Platform } from 'react-native';
import { createContext, useContext, useMemo, type ReactNode } from 'react';

import type { RelayClient } from './api';
import { useClient } from './connection';

/**
 * Downloads a project file and opens it.
 * @param client     The connected Relay client (carries the auth token).
 * @param projectId  The project the file belongs to.
 * @param path       Path relative to the project folder (e.g. "app-release.apk").
 */
export async function downloadAndOpen(
  client: RelayClient,
  projectId: string,
  path: string,
): Promise<void> {
  const url = `${client.creds.url}/projects/${encodeURIComponent(projectId)}/download?path=${encodeURIComponent(path)}`;
  const filename = path.split('/').pop()?.split('\\').pop() ?? 'file';
  const headers = { Authorization: `Bearer ${client.creds.token}` };

  if (Platform.OS === 'android') {
    await androidDownload(url, filename, headers);
  } else {
    await webDownload(url, filename, headers);
  }
}

/** Android: download to the cache, then hand it to the system handler. */
async function androidDownload(url: string, filename: string, headers: Record<string, string>): Promise<void> {
  const dir = FileSystem.cacheDirectory;
  if (!dir) throw new Error('no space for the download');
  const file = `${dir}${filename}`;
  await FileSystem.deleteAsync(file, { idempotent: true });

  const task = FileSystem.createDownloadResumable(url, file, { headers });
  const res = await task.downloadAsync();
  if (!res || res.status !== 200) throw new Error(`the download failed (HTTP ${res?.status ?? '?'})`);

  // Open with the system handler: install an APK, open a zip, view an image.
  const uri = await FileSystem.getContentUriAsync(file);
  await IntentLauncher.startActivityAsync('android.intent.action.VIEW', {
    data: uri,
    flags: 1, // FLAG_GRANT_READ_URI_PERMISSION
    type: mimeFor(filename),
  });
}

/** Desktop / web: fetch with the auth header, save via a blob link. */
async function webDownload(url: string, filename: string, headers: Record<string, string>): Promise<void> {
  const res = await fetch(url, { headers });
  if (!res.ok) throw new Error(`the download failed (HTTP ${res.status})`);
  const blob = await res.blob();
  const objectUrl = URL.createObjectURL(blob);
  const a = document.createElement('a');
  a.href = objectUrl;
  a.download = filename;
  document.body.appendChild(a);
  a.click();
  document.body.removeChild(a);
  URL.revokeObjectURL(objectUrl);
}

function mimeFor(filename: string): string {
  const ext = filename.split('.').pop()?.toLowerCase() ?? '';
  switch (ext) {
    case 'apk':
      return 'application/vnd.android.package-archive';
    case 'zip':
      return 'application/zip';
    case 'png':
      return 'image/png';
    case 'jpg':
    case 'jpeg':
      return 'image/jpeg';
    case 'gif':
      return 'image/gif';
    case 'pdf':
      return 'application/pdf';
    case 'html':
    case 'htm':
      return 'text/html';
    case 'json':
      return 'application/json';
    case 'txt':
    case 'md':
      return 'text/plain';
    default:
      return 'application/octet-stream';
  }
}

// ---------------------------------------------------------------------------
// React context so the Markdown renderer (deep in the tree) can trigger a
// download without threading props through every component.
// ---------------------------------------------------------------------------

interface DownloadValue {
  /** Download a project-relative file and open it. */
  download: (path: string) => Promise<void>;
}

const DownloadContext = createContext<DownloadValue | null>(null);

/**
 * Wraps the chat so any rendered `relay-download://<path>` link can fetch
 * the file from this project. Reads the client from the connection context.
 */
export function DownloadProvider({
  projectId,
  children,
}: {
  projectId: string | null;
  children: ReactNode;
}) {
  const client = useClient();
  const value = useMemo<DownloadValue | null>(() => {
    if (!client || !projectId) return null;
    return {
      download: (path: string) => downloadAndOpen(client, projectId, path),
    };
  }, [client, projectId]);
  return <DownloadContext.Provider value={value}>{children}</DownloadContext.Provider>;
}

/** The download handler for the current project, or null when unavailable. */
export function useDownload(): DownloadValue | null {
  return useContext(DownloadContext);
}
