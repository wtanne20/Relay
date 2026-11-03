import type { ConfigContext, ExpoConfig } from 'expo/config';

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
  slug: config.slug ?? 'relay',
  scheme: IS_DEV ? 'relay-dev' : config.scheme,
  android: {
    ...config.android,
    package: IS_DEV ? 'com.example.relay.dev' : config.android?.package,
  },
  ios: {
    ...config.ios,
    bundleIdentifier: IS_DEV ? 'com.example.relay.dev' : config.ios?.bundleIdentifier,
  },
});
