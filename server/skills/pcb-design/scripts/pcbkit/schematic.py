"""Writes a KiCad 9 schematic from a Design.

Every part is placed on one sheet and every connected pin gets a net label
with the net's name, so the schematic is driven entirely by the design file
(no hand-drawn wires to keep in sync). KiCad's ERC then checks it like any
other schematic, and the netlist KiCad exports from it is what the board is
built from.
"""

import datetime
import uuid as _uuid

from . import sexpr
from .sexpr import Q

GRID = 2.54


def _uid():
    return Q(str(_uuid.uuid4()))


def _snap(v):
    return round(round(v / GRID) * GRID, 4)


def _font(size=1.27, hide=False, justify=None):
    eff = ["effects", ["font", ["size", size, size]]]
    if justify:
        eff.append(["justify"] + justify)
    if hide:
        eff.append(["hide", "yes"])
    return eff


def _prop(name, value, x, y, hide=False, angle=0, justify=None):
    return ["property", Q(name), Q(value), ["at", x, y, angle], _font(hide=hide, justify=justify)]


def _label_for(netname, x, y, pin_angle):
    # A pin's angle points from its connection point into the symbol body,
    # so the label goes the other way.
    out = {0: 180, 90: 270, 180: 0, 270: 90}[int(round(pin_angle)) % 360]
    just = ["left", "bottom"] if out in (0, 90) else ["right", "bottom"]
    return ["label", Q(netname), ["at", x, y, out], _font(justify=just), ["uuid", _uid()]]


def build(design, project_name, sheet_uuid=None):
    """Returns (schematic text, {ref: symbol uuid}) for the design."""
    libs = design.libs
    sheet_uuid = sheet_uuid or str(_uuid.uuid4())
    pin_net = {}
    for net, pins in design.nets.items():
        for p in pins:
            pin_net[(p.part.ref, p.number)] = net
    nc = {(p.part.ref, p.number) for p in design.no_connects}

    lib_ids = []
    items = []  # (part, unit) to place
    for part in design.parts:
        if part.symbol is None:
            continue
        if part.symbol not in lib_ids:
            lib_ids.append(part.symbol)
        for u in libs.symbol_units(part.symbol):
            items.append((part, u))
    if design.power_flags and "power:PWR_FLAG" not in lib_ids:
        lib_ids.append("power:PWR_FLAG")

    # Size each block: symbol body plus room for the longest label each side.
    blocks = []
    for part, unit in items:
        x0, y0, x1, y1 = libs.symbol_bbox(part.symbol, unit)
        for p in libs.pins(part.symbol):
            if p["unit"] in (0, unit):
                x0, x1 = min(x0, p["x"]), max(x1, p["x"])
                y0, y1 = min(y0, p["y"]), max(y1, p["y"])
        longest = max([len(n) for (r, _), n in pin_net.items() if r == part.ref] + [4])
        margin = longest * 1.1 + 6
        # Symbol coords are y-up; convert to y-down extents around origin.
        blocks.append(
            dict(part=part, unit=unit, left=-x0 + margin, right=x1 + margin, top=y1 + 6, bottom=-y0 + 6)
        )

    # Shelf packing on the smallest paper size that fits.
    def pack(page_w):
        x, y, row_h = 20.0, 25.0, 0.0
        placed = []
        for b in blocks:
            w = b["left"] + b["right"]
            h = b["top"] + b["bottom"]
            if x + w > page_w - 15 and x > 20:
                x, y, row_h = 20.0, y + row_h, 0.0
            placed.append((b, _snap(x + b["left"]), _snap(y + b["top"])))
            x += w
            row_h = max(row_h, h)
        return placed, y + row_h

    paper = None
    for name, w, h in (("A4", 297, 210), ("A3", 420, 297), ("A2", 594, 420), ("A1", 841, 594), ("A0", 1189, 841)):
        placed, used_h = pack(w)
        if used_h <= h - 40:
            paper = name
            break
    if paper is None:
        paper = "A0"

    root = [
        "kicad_sch",
        ["version", "20250114"],
        ["generator", Q("pcbkit")],
        ["generator_version", Q("9.0")],
        ["uuid", Q(sheet_uuid)],
        ["paper", Q(paper)],
        [
            "title_block",
            ["title", Q(design.name)],
            ["date", Q(datetime.date.today().isoformat())],
            ["comment", 1, Q("Generated from the design file by pcbkit. Edit the design file, not this sheet.")],
        ],
        ["lib_symbols"] + [libs.symbol(i) for i in lib_ids],
    ]

    sym_uuids = {}
    graphics = []
    symbols = []
    for b, ox, oy in placed:
        part, unit = b["part"], b["unit"]
        su = sym_uuids.setdefault(part.ref, str(_uuid.uuid4()))
        # Multi-unit parts share one uuid per unit in KiCad; give extra units
        # their own.
        inst_uuid = su if unit == libs.symbol_units(part.symbol)[0] else str(_uuid.uuid4())
        x0, y0, x1, y1 = libs.symbol_bbox(part.symbol, unit)
        upins = [p for p in libs.pins(part.symbol) if p["unit"] in (0, unit)]
        if upins and all(int(p["angle"]) % 180 == 90 for p in upins):
            # Two-terminal parts drawn vertically: text to the right.
            rx = ox + x1 + 1.27
            ref_xy, val_xy, just = (rx, oy - 1.27), (rx, oy + 1.27), ["left"]
        else:
            def label_len(p):
                return len(pin_net.get((part.ref, p["number"]), "")) * 1.27 + 1.27

            top = max([y1] + [p["y"] + (label_len(p) if int(p["angle"]) == 270 else 0) for p in upins])
            bot = min([y0] + [p["y"] - (label_len(p) if int(p["angle"]) == 90 else 0) for p in upins])
            ref_xy, val_xy, just = (ox, _snap(oy - top - 2.54)), (ox, _snap(oy - bot + 2.54)), None
        props = [
            _prop("Reference", part.ref, ref_xy[0], ref_xy[1], justify=just),
            _prop("Value", part.value, val_xy[0], val_xy[1], justify=just),
            _prop("Footprint", part.footprint, ox, oy, hide=True),
            _prop("Datasheet", libs.symbol_property(part.symbol, "Datasheet"), ox, oy, hide=True),
            _prop("Description", libs.symbol_property(part.symbol, "Description"), ox, oy, hide=True),
        ]
        extra = dict(part.fields)
        if part.lcsc:
            extra["LCSC"] = part.lcsc
        if part.mpn:
            extra["MPN"] = part.mpn
        for k, v in extra.items():
            props.append(_prop(k, v, ox, oy, hide=True))
        pins = [p for p in libs.pins(part.symbol) if p["unit"] in (0, unit)]
        sym = [
            "symbol",
            ["lib_id", Q(part.symbol)],
            ["at", ox, oy, 0],
            ["unit", unit],
            ["exclude_from_sim", "no"],
            ["in_bom", "no" if part.dnp else "yes"],
            ["on_board", "yes"],
            ["dnp", "yes" if part.dnp else "no"],
            ["uuid", Q(inst_uuid)],
        ] + props
        for p in pins:
            sym.append(["pin", Q(p["number"]), ["uuid", _uid()]])
        sym.append(
            ["instances", ["project", Q(project_name), ["path", Q("/" + sheet_uuid), ["reference", Q(part.ref)], ["unit", unit]]]]
        )
        symbols.append(sym)
        for p in pins:
            px, py = round(ox + p["x"], 4), round(oy - p["y"], 4)
            key = (part.ref, p["number"])
            if key in pin_net:
                graphics.append(_label_for(pin_net[key], px, py, p["angle"]))
            elif key in nc:
                graphics.append(["no_connect", ["at", px, py], ["uuid", _uid()]])

    # Power flags along the bottom of the sheet.
    page_h = {"A4": 210, "A3": 297, "A2": 420, "A1": 594, "A0": 841}[paper]
    fx = 25.4
    for i, net in enumerate(design.power_flags):
        ref = "#FLG%02d" % (i + 1)
        x, y = _snap(fx), _snap(page_h - 30)
        symbols.append(
            [
                "symbol",
                ["lib_id", Q("power:PWR_FLAG")],
                ["at", x, y, 0],
                ["unit", 1],
                ["exclude_from_sim", "no"],
                ["in_bom", "yes"],
                ["on_board", "yes"],
                ["dnp", "no"],
                ["uuid", _uid()],
                _prop("Reference", ref, x, y - 3.81, hide=True),
                _prop("Value", "PWR_FLAG", x, y - 3.81),
                _prop("Footprint", "", x, y, hide=True),
                _prop("Datasheet", "", x, y, hide=True),
                _prop("Description", "", x, y, hide=True),
                ["pin", Q("1"), ["uuid", _uid()]],
                ["instances", ["project", Q(project_name), ["path", Q("/" + sheet_uuid), ["reference", Q(ref)], ["unit", 1]]]],
            ]
        )
        graphics.append(_label_for(net, x, y, 90))
        fx += max(20.0, len(net) * 1.3 + 10)

    root += graphics + symbols
    root.append(["sheet_instances", ["path", Q("/"), ["page", Q("1")]]])
    root.append(["embedded_fonts", "no"])
    return sexpr.dump(root) + "\n", sym_uuids, sheet_uuid
