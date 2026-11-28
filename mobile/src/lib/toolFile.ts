/**
 * Your own tools as a file, to share them or move them to another computer.
 * Export writes every tool (without its on/off state); import reads a file
 * like that, a bare list of tools, or a single tool.
 */
import * as DocumentPicker from 'expo-document-picker';
import * as FileSystem from 'expo-file-system/legacy';
import { Platform } from 'react-native';

import type { CustomTool } from './types';

export const FILE_KIND = 'relay-tools';

export interface ToolFile {
  kind: typeof FILE_KIND;
  version: 1;
  /** The shell the tools were written for, e.g. powershell.exe. */
  shell: string;
  exported_at: string;
  tools: Omit<CustomTool, 'enabled'>[];
}

export interface ParsedToolFile {
  tools: CustomTool[];
  /** The shell they were written for, when the file says. */
  shell: string | null;
}

export function toolFile(tools: CustomTool[], shell: string): string {
  const file: ToolFile = {
    kind: FILE_KIND,
    version: 1,
    shell,
    exported_at: new Date().toISOString(),
    tools: tools.map(({ enabled: _enabled, ...t }) => t),
  };
  return JSON.stringify(file, null, 2) + '\n';
}

const str = (v: unknown) => (typeof v === 'string' ? v : '');

/** Reads a tools file. Throws a readable message when it isn't one. */
export function parseToolFile(text: string): ParsedToolFile {
  let data: unknown;
  try {
    data = JSON.parse(text);
  } catch {
    throw new Error("This isn't a tools file. Export one from Settings > Tools in Relay.");
  }
  const obj = data && typeof data === 'object' && !Array.isArray(data) ? (data as Record<string, unknown>) : null;
  const list: unknown[] = Array.isArray(data)
    ? data
    : Array.isArray(obj?.tools)
      ? (obj.tools as unknown[])
      : obj && 'command' in obj
        ? [obj]
        : [];
  const tools = list
    .filter((t): t is Record<string, unknown> => !!t && typeof t === 'object')
    .map<CustomTool>((t) => ({
      name: str(t.name).trim(),
      description: str(t.description),
      command: str(t.command),
      parameters: (Array.isArray(t.parameters) ? t.parameters : [])
        .filter((p): p is Record<string, unknown> => !!p && typeof p === 'object')
        .map((p) => ({ name: str(p.name).trim(), description: str(p.description), required: p.required !== false })),
      enabled: false,
      timeout_seconds:
        typeof t.timeout_seconds === 'number' && Number.isFinite(t.timeout_seconds) ? Math.max(0, Math.round(t.timeout_seconds)) : 0,
    }))
    .filter((t) => t.name || t.command);
  if (!tools.length) throw new Error('No tools found in this file.');
  return { tools, shell: typeof obj?.shell === 'string' && obj.shell ? obj.shell : null };
}

/**
 * Saves the file: a download on the desktop, and on Android a folder the
 * user picks (Downloads, for example). Returns false when they cancel.
 */
export async function saveToolFile(text: string, filename = 'relay-tools.json'): Promise<boolean> {
  if (Platform.OS === 'web') {
    const url = URL.createObjectURL(new Blob([text], { type: 'application/json' }));
    const a = document.createElement('a');
    a.href = url;
    a.download = filename;
    document.body.appendChild(a);
    a.click();
    document.body.removeChild(a);
    setTimeout(() => URL.revokeObjectURL(url), 1000);
    return true;
  }
  const saf = FileSystem.StorageAccessFramework;
  const perm = await saf.requestDirectoryPermissionsAsync();
  if (!perm.granted) return false;
  const uri = await saf.createFileAsync(perm.directoryUri, filename.replace(/\.json$/, ''), 'application/json');
  await FileSystem.writeAsStringAsync(uri, text);
  return true;
}

/** Lets the user pick a tools file and returns its text, or null when they cancel. */
export async function pickToolFile(): Promise<string | null> {
  if (Platform.OS === 'web') {
    return new Promise((resolve, reject) => {
      const input = document.createElement('input');
      input.type = 'file';
      input.accept = '.json,application/json';
      input.addEventListener('cancel', () => resolve(null));
      input.onchange = () => {
        const f = input.files?.[0];
        if (!f) return resolve(null);
        if (f.size > 2_000_000) return reject(new Error('That file is too big to be a tools file.'));
        f.text().then(resolve, reject);
      };
      input.click();
    });
  }
  const res = await DocumentPicker.getDocumentAsync({ copyToCacheDirectory: true, type: ['application/json', 'text/*', '*/*'] });
  if (res.canceled || !res.assets[0]) return null;
  const a = res.assets[0];
  if (a.size && a.size > 2_000_000) throw new Error('That file is too big to be a tools file.');
  return FileSystem.readAsStringAsync(a.uri);
}
