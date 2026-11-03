import { Platform, useWindowDimensions, type ViewStyle } from 'react-native';

/** Window width from which the desktop layout (sidebar + main pane) is used. */
export const WIDE_MIN = 900;
/** Comfortable reading width for chat and settings on wide windows. */
export const PAGE_MAX = 820;

/**
 * True for the desktop layout: the Windows app (or a wide browser window
 * during development). Phones always get the phone layout.
 */
export function useWide(): boolean {
  const { width } = useWindowDimensions();
  return Platform.OS === 'web' && width >= WIDE_MIN;
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
