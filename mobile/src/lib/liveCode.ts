/**
 * Helpers for showing file changes: reading a tool call's arguments while
 * the model is still writing them, and turning a change into lines to show.
 */

/**
 * The string fields of a JSON object that may be cut off part way, such as
 * `{"path":"a.ts","content":"line 1\nli`. The last string is returned as far
 * as it got. Fields that aren't strings are skipped.
 */
export function partialArgs(text: string): Record<string, string> {
  const out: Record<string, string> = {};
  let i = text.indexOf('{');
  if (i < 0) return out;
  i++;
  const n = text.length;
  const ws = () => {
    while (i < n && /\s/.test(text[i])) i++;
  };
  // Reads a string starting after its opening quote. Returns it and whether
  // it was closed.
  const str = (): [string, boolean] => {
    let s = '';
    let start = i;
    while (i < n) {
      const c = text[i];
      if (c === '"') {
        s += text.slice(start, i);
        i++;
        return [s, true];
      }
      if (c === '\\') {
        s += text.slice(start, i);
        if (i + 1 >= n) return [s, false];
        const e = text[i + 1];
        if (e === 'u') {
          if (i + 6 > n) return [s, false];
          const code = Number.parseInt(text.slice(i + 2, i + 6), 16);
          s += Number.isNaN(code) ? '' : String.fromCharCode(code);
          i += 6;
        } else {
          s += ESC[e] ?? e;
          i += 2;
        }
        start = i;
        continue;
      }
      i++;
    }
    s += text.slice(start, i);
    return [s, false];
  };
  // Skips a value that isn't a string (number, bool, object, array).
  const skip = () => {
    let depth = 0;
    while (i < n) {
      const c = text[i];
      if (c === '"') {
        i++;
        str();
        continue;
      }
      if (c === '{' || c === '[') depth++;
      else if (c === '}' || c === ']') {
        if (depth === 0) return;
        depth--;
      } else if (c === ',' && depth === 0) return;
      i++;
    }
  };
  for (;;) {
    ws();
    if (i >= n || text[i] === '}') return out;
    if (text[i] === ',') {
      i++;
      continue;
    }
    if (text[i] !== '"') return out;
    i++;
    const [key, keyDone] = str();
    if (!keyDone) return out;
    ws();
    if (text[i] !== ':') return out;
    i++;
    ws();
    if (i >= n) return out;
    if (text[i] === '"') {
      i++;
      const [value, done] = str();
      out[key] = value;
      if (!done) return out;
    } else {
      skip();
    }
  }
}

const ESC: Record<string, string> = { n: '\n', t: '\t', r: '\r', b: '\b', f: '\f', '"': '"', '\\': '\\', '/': '/' };

export type Line = { kind: 'add' | 'del' | 'same'; text: string };

function splitLines(s: string): string[] {
  if (!s) return [];
  const lines = s.replace(/\r\n/g, '\n').split('\n');
  if (lines[lines.length - 1] === '') lines.pop();
  return lines;
}

/**
 * Lines for replacing `before` with `after`: lines both share at the start
 * and end are shown as context, the rest as removed then added.
 */
export function changeLines(before: string, after: string): Line[] {
  const a = splitLines(before);
  const b = splitLines(after);
  let head = 0;
  while (head < a.length && head < b.length && a[head] === b[head]) head++;
  let tail = 0;
  while (tail < a.length - head && tail < b.length - head && a[a.length - 1 - tail] === b[b.length - 1 - tail]) tail++;
  return [
    ...a.slice(0, head).map((text) => ({ kind: 'same' as const, text })),
    ...a.slice(head, a.length - tail).map((text) => ({ kind: 'del' as const, text })),
    ...b.slice(head, b.length - tail).map((text) => ({ kind: 'add' as const, text })),
    ...a.slice(a.length - tail).map((text) => ({ kind: 'same' as const, text })),
  ];
}

export type FileChange = {
  tool: 'write_file' | 'edit_file';
  path: string;
  lines: Line[];
  added: number;
  removed: number;
};

/**
 * What a write_file or edit_file call changes, from its (possibly partial)
 * arguments. Null for other tools. While an edit is being written, its new
 * text only starts once the old text is complete.
 */
export function fileChange(name: string, args: Record<string, string>): FileChange | null {
  if (name !== 'write_file' && name !== 'edit_file') return null;
  const lines =
    name === 'write_file'
      ? splitLines(args.content ?? '').map((text) => ({ kind: 'add' as const, text }))
      : 'new_text' in args
        ? changeLines(args.old_text ?? '', args.new_text)
        : splitLines(args.old_text ?? '').map((text) => ({ kind: 'del' as const, text }));
  return {
    tool: name,
    path: args.path ?? '',
    lines,
    added: lines.filter((l) => l.kind === 'add').length,
    removed: lines.filter((l) => l.kind === 'del').length,
  };
}

/** Parses complete arguments, falling back to the partial reader. */
export function readArgs(text: string): Record<string, string> {
  try {
    const v = JSON.parse(text || '{}');
    if (v && typeof v === 'object' && !Array.isArray(v)) {
      const out: Record<string, string> = {};
      for (const [k, x] of Object.entries(v)) if (typeof x === 'string') out[k] = x;
      return out;
    }
  } catch {
    // Still being written, or not JSON: read what's there.
  }
  return partialArgs(text);
}
