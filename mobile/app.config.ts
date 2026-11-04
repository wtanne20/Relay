import type { ConfigContext, ExpoConfig } from 'expo/config';

import pkg from './package.json';

/** The app's version follows package.json; versionCode 1.2.3 -> 10203. */
const [major = 0, minor = 0, patch = 0] = pkg.version.split('.').map((n) => Number.parseInt(n, 10) || 0);
const VERSION_CODE = major * 10000 + minor * 100 + patch;

/**
 * APP_VARIANT=dev builds "Relay Dev": a development build with its own app id,
 * so it installs next to the normal Relay app. It loads the app's code live
 * from the Metro bundler on your PC (see DEVELOPING.md), so changes show up
 * on the phone without building a new APK.
 */
const IS_DEV = process.env.APP_VARIANT === 'dev';

export default ({ config }: ConfigContext): ExpoConfig => ({
  ...config,
  name: IS_DEV ? 'Relay Dev' : (config.name ?? 'Relay'),
  version: pkg.version,
  slug: config.slug ?? 'relay',
  scheme: IS_DEV ? 'relay-dev' : config.scheme,
  android: {
    ...config.android,
    package: IS_DEV ? 'com.example.relay.dev' : config.android?.package,
    versionCode: VERSION_CODE,
  },
  ios: {
    ...config.ios,
    bundleIdentifier: IS_DEV ? 'com.example.relay.dev' : config.ios?.bundleIdentifier,
  },
});
