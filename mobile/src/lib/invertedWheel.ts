// Mouse wheel and trackpad scrolling for an inverted FlatList on the web
// (the browser build and the desktop app).
//
// An inverted list is drawn upside down (scaleY(-1)), so react-native-web
// flips wheel events by hand. Its handler caps the new scroll position at
// the total height of the rows it has measured. Padding, the "loading older"
// spinner and rows that grew while a reply streamed in aren't counted, so
// once you've scrolled far enough up the cap is below where you are, and the
// next turn of the wheel jumps the view back down: scrolling "reverses".
// It also scrolls whatever element is under the pointer, which breaks code
// blocks that scroll sideways.
//
// This handler runs first (capture phase) and stops react-native-web's:
// nested scrollers (code blocks, tables) scroll natively, and everything else
// moves the list by exactly the wheel's distance in the direction you
// turned it.

const LINE_HEIGHT = 16;

function canScroll(el: HTMLElement, dx: number, dy: number): boolean {
  const style = getComputedStyle(el);
  if (dy !== 0 && /(auto|scroll)/.test(style.overflowY) && el.scrollHeight > el.clientHeight) {
    if (dy < 0 ? el.scrollTop > 0 : el.scrollTop + el.clientHeight < el.scrollHeight - 1) return true;
  }
  if (dx !== 0 && /(auto|scroll)/.test(style.overflowX) && el.scrollWidth > el.clientWidth) {
    if (dx < 0 ? el.scrollLeft > 0 : el.scrollLeft + el.clientWidth < el.scrollWidth - 1) return true;
  }
  return false;
}

export function attachInvertedWheel(node: HTMLElement): () => void {
  const onWheel = (ev: WheelEvent) => {
    // Keep react-native-web's own inverted handler out of it in every case.
    ev.stopImmediatePropagation();
    // Pinch-to-zoom on trackpads arrives as a wheel event with ctrl held.
    if (ev.ctrlKey) return;
    const unit = ev.deltaMode === 1 ? LINE_HEIGHT : ev.deltaMode === 2 ? node.clientHeight : 1;
    const dx = ev.deltaX * unit;
    const dy = ev.deltaY * unit;
    // Something inside the list that can scroll this way gets the wheel.
    for (let el = ev.target as HTMLElement | null; el && el !== node; el = el.parentElement) {
      if (canScroll(el, dx, dy)) return;
    }
    // Mostly sideways: nothing for the list to do.
    if (Math.abs(dy) <= Math.abs(dx)) return;
    // The list is flipped, so wheel down (towards newer) lowers scrollTop.
    node.scrollTop -= dy;
    ev.preventDefault();
  };
  node.addEventListener('wheel', onWheel, { capture: true, passive: false });
  return () => node.removeEventListener('wheel', onWheel, { capture: true });
}
