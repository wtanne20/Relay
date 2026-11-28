import { useEffect, useRef, useState } from 'react';

/** Slowest reveal, in characters per second, so a short reply still flows. */
const MIN_RATE = 90;
/** How far behind the model the text may fall: the backlog is shown within this. */
const CATCH_UP_MS = 280;
/** Redraws at most this often; each one re-renders the Markdown. */
const FRAME_MS = 32;

const now = () => (typeof performance !== 'undefined' ? performance.now() : Date.now());

/**
 * The model's text sent in bursts (a few tokens per WebSocket frame, with
 * pauses between), revealed at an even pace instead, like Claude chat: the
 * shown text chases the real text, faster the further behind it is, so it
 * never lags more than a fraction of a second.
 *
 * `live` only matters when the text first appears: live text starts empty
 * and flows in, while text that was already there (history, a reload) is
 * shown whole. Once flowing, it keeps up with the text until it's all shown,
 * even after the reply finishes.
 */
export function useSmoothText(text: string, live: boolean): string {
  const [shown, setShown] = useState(() => (live ? 0 : text.length));
  const shownRef = useRef(shown);
  const textRef = useRef(text);
  textRef.current = text;

  // The text was replaced rather than added to: show it as it is.
  const prev = useRef(text);
  if (!text.startsWith(prev.current.slice(0, shownRef.current))) {
    shownRef.current = text.length;
  }
  prev.current = text;

  const behind = shownRef.current < text.length;
  useEffect(() => {
    if (!behind) return;
    let last = now();
    let budget = 0;
    let timer: ReturnType<typeof setTimeout> | null = null;
    let frame: number | null = null;
    const tick = () => {
      frame = null;
      const t = now();
      const dt = Math.min(t - last, 100);
      last = t;
      const target = textRef.current.length;
      const backlog = target - shownRef.current;
      if (backlog <= 0) return;
      const rate = Math.max(MIN_RATE, (backlog * 1000) / CATCH_UP_MS);
      budget += (rate * dt) / 1000;
      const step = Math.floor(budget);
      if (step > 0) {
        budget -= step;
        let next = Math.min(target, shownRef.current + step);
        // End on a whole word when one is close, so words don't spell out.
        const space = textRef.current.slice(next, next + 12).search(/\s/);
        if (space > 0 && next + space <= target) next += space;
        shownRef.current = next;
        setShown(next);
        if (next >= target) return;
      }
      timer = setTimeout(schedule, FRAME_MS);
    };
    const schedule = () => {
      timer = null;
      frame = typeof requestAnimationFrame !== 'undefined' ? requestAnimationFrame(tick) : null;
      if (frame === null) tick();
    };
    schedule();
    return () => {
      if (timer) clearTimeout(timer);
      if (frame !== null && typeof cancelAnimationFrame !== 'undefined') cancelAnimationFrame(frame);
    };
    // Restarts only when it falls behind again, not on every token.
  }, [behind]);

  return shownRef.current >= text.length ? text : text.slice(0, shownRef.current);
}

/**
 * Hides Markdown that's only half written at the end of streaming text, so
 * "**bold" or "[link](htt" doesn't flash as raw symbols before it closes.
 */
export function trimPartialMarkdown(text: string): string {
  const line = text.slice(text.lastIndexOf('\n') + 1);
  const before = text.slice(0, text.length - line.length);
  const fences = (before.match(/^\s*(```|~~~)/gm) ?? []).length;
  // A fence line still being typed ("``" or "```py") stays hidden for now.
  if (/^\s*(`{1,3}|~{1,3})[\w+#.-]*$/.test(line) && !/^\s*`[^`]+$/.test(line)) return before;
  // Inside a code block everything is shown as it is.
  if (fences % 2 === 1) return text;
  // A list marker with nothing after it yet.
  if (/^\s*([*+-]|\d+[.)])\s?$/.test(line)) return before;
  // An unfinished link: show its label until the link is complete.
  let t = text.replace(/!?\[([^\]\n]*)\]?(\([^)\s]*)?$/, '$1');
  // Inline code or bold opened on this line and not closed yet: close it.
  const last = t.slice(t.lastIndexOf('\n') + 1);
  if ((last.match(/`/g) ?? []).length % 2 === 1) return `${t}\``;
  const bold = (last.replace(/`[^`]*`/g, '').match(/\*\*/g) ?? []).length;
  if (bold % 2 === 1) t = /\*\*$/.test(t) ? t.slice(0, -2) : `${t.replace(/\*$/, '')}**`;
  return t;
}
