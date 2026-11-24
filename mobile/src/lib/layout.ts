import { Platform, useWindowDimensions, type ViewStyle } from 'react-native';

import { isDesktop } from './desktop';

/** Window width from which the desktop layout (sidebar + main pane) is used. */
export const WIDE_MIN = 900;
/** Comfortable reading width for chat and settings on wide windows. */
export const PAGE_MAX = 820;

/** Below this width the desktop sidebar is drawn narrower to leave room for the page. */
export const SIDEBAR_COMPACT_BELOW = 1000;

/**
 * True for the desktop layout: always in the desktop app, whatever the
 * window size, so the sidebar never disappears from a small window; in a
 * browser (during development) from WIDE_MIN. Phones always get the phone
 * layout.
 */
export function useWide(): boolean {
  const { width } = useWindowDimensions();
  return Platform.OS === 'web' && (isDesktop || width >= WIDE_MIN);
}

/** Centers page content at a readable width on wide windows. */
export function usePageStyle(max: number = PAGE_MAX): ViewStyle | undefined {
  const wide = useWide();
  return wide ? { width: '100%', maxWidth: max, alignSelf: 'center' } : undefined;
}

/**
 * Desktop composer behaviour: Enter sends, Shift+Enter adds a new line.
 * Returns an onKeyPress handler for a multiline TextInput (no-op on phones,
 * where Enter always adds a new line).
 */
export function enterToSend(send: () => void) {
  return (e: { nativeEvent: { key: string }; preventDefault?: () => void }) => {
    if (Platform.OS !== 'web') return;
    const ne = e.nativeEvent as { key: string; shiftKey?: boolean; isComposing?: boolean };
    if (ne.key === 'Enter' && !ne.shiftKey && !ne.isComposing) {
      e.preventDefault?.();
      send();
    }
  };
}
