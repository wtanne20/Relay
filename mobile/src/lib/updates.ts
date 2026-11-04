/**
 * Self-updates. The PC asks GitHub for Relay's latest release. On Windows the
 * PC downloads the installer, checks it and runs it (Relay closes and opens
 * again). On Android the phone downloads the new APK and hands it to the
 * system installer, which asks the person to confirm.
 */
import * as Application from 'expo-application';
import Constants from 'expo-constants';
import * as FileSystem from 'expo-file-system/legacy';
import * as IntentLauncher from 'expo-intent-launcher';
import { useCallback, useEffect, useRef, useState } from 'react';
import { Linking, Platform } from 'react-native';

import { describeError, type RelayClient } from './api';
import { isDesktop } from './desktop';
import type { ReleaseAsset, UpdateInfo } from './types';

export type UpdateKind = 'desktop' | 'android' | 'other';

export const updateKind: UpdateKind = isDesktop ? 'desktop' : Platform.OS === 'android' ? 'android' : 'other';

/** This app's own version. */
export function appVersion(): string {
  return Application.nativeApplicationVersion ?? Constants.expoConfig?.version ?? '0.0.0';
}

const isDevApp = (Application.applicationId ?? '').endsWith('.dev');

function parts(v: string): number[] {
  return v
    .replace(/^v/i, '')
    .split('-')[0]
    .split('.')
    .map((p) => Number.parseInt(p, 10) || 0);
}

/** True when `a` is a later version than `b`. */
export function newer(a: string, b: string): boolean {
  const x = parts(a);
  const y = parts(b);
  for (let i = 0; i < Math.max(x.length, y.length); i++) {
    const d = (x[i] ?? 0) - (y[i] ?? 0);
    if (d !== 0) return d > 0;
  }
  return false;
}

/** The installer this app would use from the latest release. */
export function assetFor(info: UpdateInfo | null): ReleaseAsset | null {
  const r = info?.latest;
  if (!r) return null;
  if (updateKind === 'desktop') return r.windows;
  if (updateKind === 'android') return isDevApp ? r.android_dev : r.android;
  return null;
}

/** Whether this app (or, on desktop, this PC's Relay) has a newer version. */
export function updateAvailable(info: UpdateInfo | null): boolean {
  if (!info?.latest) return false;
  if (updateKind === 'android') return newer(info.latest.version, appVersion()) && !!assetFor(info);
  if (updateKind === 'desktop') return info.available && !!assetFor(info);
  return info.available;
}

export type Phase =
  | { kind: 'idle' }
  | { kind: 'downloading'; progress: number }
  | { kind: 'installing' }
  | { kind: 'error'; message: string };

export function useUpdates(client: RelayClient | null) {
  const [info, setInfo] = useState<UpdateInfo | null>(null);
  const [checking, setChecking] = useState(false);
  const [phase, setPhase] = useState<Phase>({ kind: 'idle' });
  const alive = useRef(true);
  useEffect(
    () => () => {
      alive.current = false;
    },
    [],
  );

  const check = useCallback(
    async (refresh = false) => {
      if (!client) return;
      setChecking(true);
      try {
        const i = await client.updates(refresh);
        if (alive.current) setInfo(i);
      } catch {
        // An older Relay on the PC has no updates endpoint; say nothing.
      } finally {
        if (alive.current) setChecking(false);
      }
    },
    [client],
  );

  useEffect(() => {
    check(false);
  }, [check]);

  const installDesktop = async () => {
    if (!client) return;
    await client.installUpdate();
    // Follow the PC's progress until the installer takes over.
    for (;;) {
      await new Promise((r) => setTimeout(r, 1000));
      if (!alive.current) return;
      let i: UpdateInfo;
      try {
        i = await client.updates();
      } catch {
        // Relay closed for the installer.
        setPhase({ kind: 'installing' });
        return;
      }
      const s = i.install;
      if (s.state === 'downloading') {
        setPhase({ kind: 'downloading', progress: s.total_bytes ? s.done_bytes / s.total_bytes : 0 });
      } else if (s.state === 'installing') {
        setPhase({ kind: 'installing' });
        return;
      } else if (s.state === 'failed') {
        setPhase({ kind: 'error', message: s.error });
        return;
      }
    }
  };

  const installAndroid = async (asset: ReleaseAsset) => {
    const dir = FileSystem.cacheDirectory;
    if (!dir) throw new Error('no space for the download');
    const file = `${dir}relay-update.apk`;
    await FileSystem.deleteAsync(file, { idempotent: true });
    const task = FileSystem.createDownloadResumable(asset.url, file, {}, (p) => {
      const total = p.totalBytesExpectedToWrite || asset.size;
      if (alive.current) setPhase({ kind: 'downloading', progress: total ? p.totalBytesWritten / total : 0 });
    });
    const res = await task.downloadAsync();
    if (!res || res.status !== 200) throw new Error(`the download failed (HTTP ${res?.status ?? '?'})`);
    const info = await FileSystem.getInfoAsync(file);
    if (!info.exists || (asset.size && info.size !== asset.size)) throw new Error('the download was incomplete');
    const uri = await FileSystem.getContentUriAsync(file);
    setPhase({ kind: 'installing' });
    await IntentLauncher.startActivityAsync('android.intent.action.VIEW', {
      data: uri,
      flags: 1, // FLAG_GRANT_READ_URI_PERMISSION
      type: 'application/vnd.android.package-archive',
    });
    // Back from the installer (installed or cancelled).
    if (alive.current) setPhase({ kind: 'idle' });
  };

  const install = async () => {
    const asset = assetFor(info);
    setPhase({ kind: 'downloading', progress: 0 });
    try {
      if (updateKind === 'desktop') await installDesktop();
      else if (updateKind === 'android' && asset) await installAndroid(asset);
      else {
        if (info?.latest?.page) await Linking.openURL(info.latest.page);
        setPhase({ kind: 'idle' });
      }
    } catch (e) {
      if (alive.current) setPhase({ kind: 'error', message: describeError(e) });
    }
  };

  return { info, checking, phase, check, install, available: updateAvailable(info) };
}
