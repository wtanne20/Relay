"""Places parts by their real size, and sizes the board.

Pure Python (no KiCad needed), so it runs before the board is built. Every
part's outline comes from its footprint's courtyard (see
Libraries.footprint_box), so a screw terminal takes the space a screw
terminal takes, wherever its origin happens to be.

- Parts with `at=` stay exactly where they are.
- Parts with `center=` are moved so their outline is centred there.
- Parts with `edge=` sit flush against that board edge.
- Every other part is placed automatically: next to the pins it connects
  to (a decoupling cap lands by its chip's power pin), at least
  `design.spacing` mm from every other part, inside the board and out of
  keepouts.
- With no board size, the board is sized from the parts' total area and
  grown until everything fits.
"""

import math

GROUND = ("GND", "AGND", "DGND", "PGND", "GNDA", "GNDD", "VSS")
EDGE_MARGIN = 0.5  # mm between an ordinary part's outline and the board edge
GRID = 0.5


class NoRoom(Exception):
    pass


def rotate(x, y, rot):
    """A footprint-local point at the footprint's rotation (KiCad: degrees
    counter-clockwise on screen, y down)."""
    a = math.radians(rot)
    c, s = math.cos(a), math.sin(a)
    return x * c + y * s, -x * s + y * c


def local_box(part, rot):
    """The part's outline relative to its origin at `rot`."""
    x0, y0, x1, y1 = part.design.libs.footprint_box(part.footprint)
    pts = [rotate(x, y, rot) for x, y in ((x0, y0), (x1, y0), (x0, y1), (x1, y1))]
    xs, ys = [p[0] for p in pts], [p[1] for p in pts]
    box = (min(xs), min(ys), max(xs), max(ys))
    if part.side == "bottom":
        box = (box[0], -box[3], box[2], -box[1])
    return box


def local_keepclear(part, rot):
    out = []
    for x0, y0, x1, y1 in part.design.libs.footprint_keepclear(part.footprint):
        pts = [rotate(x, y, rot) for x, y in ((x0, y0), (x1, y0), (x0, y1), (x1, y1))]
        xs, ys = [p[0] for p in pts], [p[1] for p in pts]
        b = (min(xs), min(ys), max(xs), max(ys))
        if part.side == "bottom":
            b = (b[0], -b[3], b[2], -b[1])
        out.append(b)
    return out


def _shift(box, x, y):
    return (box[0] + x, box[1] + y, box[2] + x, box[3] + y)


def board_box(part):
    lx0, ly0, lx1, ly1 = local_box(part, part.rot)
    return (part.at[0] + lx0, part.at[1] + ly0, part.at[0] + lx1, part.at[1] + ly1)


def pad_xy(part, number):
    xy = part.design.libs.footprint_pad_xy(part.footprint).get(str(number))
    if xy is None:
        return None
    x, y = rotate(xy[0], xy[1], part.rot)
    if part.side == "bottom":
        y = -y
    return part.at[0] + x, part.at[1] + y


def _overlaps(a, b, gap):
    return a[0] < b[2] + gap and b[0] < a[2] + gap and a[1] < b[3] + gap and b[1] < a[3] + gap


def _is_ground(net):
    return net.upper() in GROUND or net.upper().endswith("GND")


def _links(design):
    """{ref: {other_ref: [(weight, my_pin, other_pin)]}} over shared nets.
    Ground is left out (everything touches it); big nets count for less."""
    links = {p.ref: {} for p in design.parts}
    for net, pins in design.nets.items():
        if _is_ground(net) or len(pins) < 2:
            continue
        w = 1.0 / (len(pins) - 1)
        for a in pins:
            for b in pins:
                if a.part is not b.part:
                    links[a.part.ref].setdefault(b.part.ref, []).append((w, a.number, b.number))
    return links


def _rotations(part):
    if part.rot_given or part.symbol is None:
        return [part.rot]
    n_pins = len(part.pins)
    if part.edge:
        return [part.rot]
    return [0.0, 90.0] if n_pins <= 3 else [0.0, 90.0, 180.0, 270.0]


def _gap(design, part):
    # Chips with many pins need room for tracks to fan out.
    n = len(part.pins) if part.symbol else 0
    return design.spacing + (1.0 if n > 16 else 0.5 if n > 6 else 0.0)


def _area(part):
    x0, y0, x1, y1 = local_box(part, 0)
    return (x1 - x0) * (y1 - y0)


def _place_fixed(design):
    w, h = design.width, design.height
    inset = design.corner_hole_inset
    corners = [(inset, inset), (w - inset, inset), (inset, h - inset), (w - inset, h - inset)] if inset else []
    for p in design.parts:
        if p.corner is not None:
            p.at = corners[p.corner]
        elif p.center is not None:
            lx0, ly0, lx1, ly1 = local_box(p, p.rot)
            p.at = (round(p.center[0] - (lx0 + lx1) / 2, 3), round(p.center[1] - (ly0 + ly1) / 2, 3))


def _try_place(design, log):
    w, h = design.width, design.height
    for p in design.parts:
        if p.auto_placed:
            p.at = None
            p.auto_placed = False
    _place_fixed(design)
    placed = [p for p in design.parts if p.at is not None]
    todo = [p for p in design.parts if p.at is None]
    obstacles = []  # (part, rect, is a keep-clear area)

    def add_obstacles(p):
        obstacles.append((p, board_box(p), False))
        for k in local_keepclear(p, p.rot):
            obstacles.append((p, _shift(k, p.at[0], p.at[1]), True))

    for p in placed:
        add_obstacles(p)
    keepouts = [tuple(k["rect"]) for k in design.keepouts]
    links = _links(design)
    by_ref = {p.ref: p for p in design.parts}

    def target(part):
        sx = sy = tw = 0.0
        for other_ref, conns in links[part.ref].items():
            other = by_ref[other_ref]
            if other.at is None:
                continue
            for wgt, _mine, theirs in conns:
                xy = pad_xy(other, theirs) or ((board_box(other)[0] + board_box(other)[2]) / 2,
                                               (board_box(other)[1] + board_box(other)[3]) / 2)
                sx, sy, tw = sx + xy[0] * wgt, sy + xy[1] * wgt, tw + wgt
        if tw == 0:
            return (w / 2, h / 2), 0.0
        return (sx / tw, sy / tw), tw

    def free(box, gap, keepclear):
        if box[0] < 0 or box[1] < 0 or box[2] > w or box[3] > h:
            return False
        for other, ob, is_keep in obstacles:
            g = 0 if is_keep else (max(gap, _gap(design, other)) if other.symbol else gap)
            if _overlaps(box, ob, g):
                return False
        for k in keepouts:
            if _overlaps(box, k, 0):
                return False
        # This part's own keep-clear area may hang off the board, but must
        # not cover anything.
        for kc in keepclear:
            for other, ob, is_keep in obstacles:
                if not is_keep and _overlaps(kc, ob, 0):
                    return False
        return True

    def place(part):
        (tx, ty), _ = target(part)
        gap = _gap(design, part)
        best = None
        for rot in _rotations(part):
            lx0, ly0, lx1, ly1 = local_box(part, rot)
            bw, bh = lx1 - lx0, ly1 - ly0
            cands = []
            if part.edge in ("left", "right"):
                x = 0 if part.edge == "left" else w - bw
                cands = [(x, EDGE_MARGIN + i * GRID) for i in range(int((h - bh - 2 * EDGE_MARGIN) / GRID) + 1)]
            elif part.edge in ("top", "bottom"):
                y = 0 if part.edge == "top" else h - bh
                cands = [(EDGE_MARGIN + i * GRID, y) for i in range(int((w - bw - 2 * EDGE_MARGIN) / GRID) + 1)]
            else:
                nx = int((w - bw - 2 * EDGE_MARGIN) / GRID) + 1
                ny = int((h - bh - 2 * EDGE_MARGIN) / GRID) + 1
                cands = [(EDGE_MARGIN + i * GRID, EDGE_MARGIN + j * GRID) for i in range(max(nx, 0)) for j in range(max(ny, 0))]
            # Nearest to the target first (by the outline's centre).
            cands.sort(key=lambda c: (c[0] + bw / 2 - tx) ** 2 + (c[1] + bh / 2 - ty) ** 2)
            kcs = local_keepclear(part, rot)
            for bx, by in cands:
                box = (bx, by, bx + bw, by + bh)
                ox, oy = bx - lx0, by - ly0
                if free(box, gap, [_shift(k, ox, oy) for k in kcs]):
                    d = (bx + bw / 2 - tx) ** 2 + (by + bh / 2 - ty) ** 2
                    if best is None or d < best[0] - 1e-9:
                        best = (d, rot, (round(bx - lx0, 3), round(by - ly0, 3)), box)
                    break
        if best is None:
            raise NoRoom(part)
        part.rot, part.at = best[1], best[2]
        part.auto_placed = True
        add_obstacles(part)

    # Edge parts first, then the big chips, then whatever is most connected
    # to what's already down (so support parts gather round their chip).
    for part in sorted([p for p in todo if p.edge], key=_area, reverse=True):
        place(part)
    rest = [p for p in todo if not p.edge]
    for part in sorted([p for p in rest if p.symbol and len(p.pins) > 6], key=_area, reverse=True):
        place(part)
    rest = [p for p in rest if p.at is None]
    while rest:
        rest.sort(key=lambda p: (-target(p)[1], -_area(p)))
        place(rest.pop(0))


def _auto_size(design):
    total = 0.0
    for p in design.parts:
        x0, y0, x1, y1 = local_box(p, p.rot)
        g = _gap(design, p)
        total += (x1 - x0 + g) * (y1 - y0 + g)
    area = total * 2.0  # routing room
    w = math.sqrt(area * design.aspect)
    h = area / w
    # Parts placed by hand must fit too.
    for p in design.parts:
        if p.at is not None and p.corner is None:
            x0, y0, x1, y1 = board_box(p)
            w, h = max(w, x1 + 1), max(h, y1 + 1)
    return _up(w), _up(h)


def _up(v, step=5.0):
    return max(20.0, math.ceil(v / step) * step)


def auto_place(design, log=print):
    """Size the board (if no size was given) and place every unplaced part.
    Returns lines describing what was decided."""
    if not design.size_given:
        design.width, design.height = _auto_size(design)
    for attempt in range(12):
        try:
            _try_place(design, log)
            break
        except NoRoom as e:
            part = e.args[0]
            if design.size_given:
                x0, y0, x1, y1 = local_box(part, part.rot)
                raise NoRoom(
                    "No room for %s (%s, %.1f x %.1f mm) on the %g x %g mm board with %.1f mm spacing. "
                    "Make the board bigger (or use size=None to size it automatically), lower spacing=, "
                    "or move hand-placed parts to free space."
                    % (part.ref, part.footprint.split(":", 1)[1], x1 - x0, y1 - y0, design.width, design.height,
                       design.spacing)
                )
            design.width, design.height = _up(design.width * 1.1), _up(design.height * 1.1)
    else:
        raise NoRoom("Couldn't fit the parts even on a %g x %g mm board" % (design.width, design.height))
    return summary(design)


def summary(design):
    lines = ["board %g x %g mm%s" % (design.width, design.height, "" if design.size_given else " (sized automatically)")]
    for p in design.parts:
        x0, y0, x1, y1 = board_box(p)
        lines.append(
            "%-5s %-38s %5.1f x %-5.1f mm  at=(%g, %g) rot=%g  outline x %.1f..%.1f, y %.1f..%.1f%s"
            % (p.ref, p.footprint.split(":", 1)[1][:38], x1 - x0, y1 - y0, p.at[0], p.at[1], p.rot,
               x0, x1, y0, y1, "  [auto]" if p.auto_placed else "")
        )
    return lines


def sizes(design):
    """Each part's real outline size (no placement needed)."""
    lines = []
    total = 0.0
    for p in design.parts:
        x0, y0, x1, y1 = local_box(p, p.rot)
        total += (x1 - x0) * (y1 - y0)
        cx, cy = (x0 + x1) / 2, (y0 + y1) / 2
        off = "" if abs(cx) < 0.3 and abs(cy) < 0.3 else "  origin is NOT the middle: middle is (%+.1f, %+.1f) from at=" % (cx, cy)
        lines.append("%-5s %-40s %5.1f x %-5.1f mm at rot %g%s" % (p.ref, p.footprint.split(":", 1)[1][:40], x1 - x0, y1 - y0, p.rot, off))
    w = math.sqrt(total * 2.0 * design.aspect)
    lines.append("parts cover %.0f mm2; with routing room a board of about %g x %g mm fits them"
                 % (total, _up(w), _up(total * 2.0 / w)))
    return lines
