"""Tiny reader and writer for KiCad S-expression files.

Lists are Python lists. Quoted strings come back as `Q` (a str subclass) so
they are written back with quotes; bare atoms are plain `str`. Numbers stay
as their original text unless you put an int/float in yourself.
"""

import re


class Q(str):
    """A quoted string atom."""


_TOKEN = re.compile(r'\s*(?:(\()|(\))|"((?:[^"\\]|\\.)*)"|([^\s()"]+))', re.S)


def parse(text):
    stack = [[]]
    pos = 0
    n = len(text)
    while pos < n:
        m = _TOKEN.match(text, pos)
        if not m:
            if text[pos:].strip() == "":
                break
            raise ValueError("bad s-expression near: %r" % text[pos : pos + 40])
        pos = m.end()
        if m.group(1):
            stack.append([])
        elif m.group(2):
            done = stack.pop()
            stack[-1].append(done)
        elif m.group(3) is not None:
            s = m.group(3)
            s = s.replace('\\"', '"').replace("\\n", "\n").replace("\\\\", "\\")
            stack[-1].append(Q(s))
        else:
            stack[-1].append(m.group(4))
    if len(stack) != 1:
        raise ValueError("unbalanced parentheses")
    return stack[0][0] if len(stack[0]) == 1 else stack[0]


def _atom(x):
    if isinstance(x, Q):
        return '"' + x.replace("\\", "\\\\").replace('"', '\\"').replace("\n", "\\n") + '"'
    if isinstance(x, bool):
        return "yes" if x else "no"
    if isinstance(x, float):
        s = ("%.6f" % x).rstrip("0").rstrip(".")
        return "0" if s in ("-0", "") else s
    return str(x)


def dump(node, indent=0):
    """Serialise with one list per line once lists start nesting."""
    if not isinstance(node, list):
        return _atom(node)
    if not any(isinstance(c, list) for c in node):
        return "(" + " ".join(_atom(c) for c in node) + ")"
    pad = "\t" * (indent + 1)
    head = []
    rest = []
    for c in node:
        if not rest and not isinstance(c, list):
            head.append(_atom(c))
        else:
            rest.append(c)
    out = "(" + " ".join(head)
    for c in rest:
        out += "\n" + pad + dump(c, indent + 1)
    return out + "\n" + "\t" * indent + ")"


def find(node, key):
    """First child list whose head is `key`."""
    for c in node:
        if isinstance(c, list) and c and c[0] == key:
            return c
    return None


def find_all(node, key):
    return [c for c in node if isinstance(c, list) and c and c[0] == key]


def prop(node, name):
    for p in find_all(node, "property"):
        if len(p) > 2 and p[1] == name:
            return p
    return None


def block_end(text, start):
    """Index just past the list that opens at text[start] == '('."""
    depth = 0
    i = start
    in_str = False
    while i < len(text):
        c = text[i]
        if in_str:
            if c == "\\":
                i += 1
            elif c == '"':
                in_str = False
        elif c == '"':
            in_str = True
        elif c == "(":
            depth += 1
        elif c == ")":
            depth -= 1
            if depth == 0:
                return i + 1
        i += 1
    raise ValueError("unterminated list")


def read_text(path):
    with open(path, encoding="utf-8") as f:
        return f.read()
