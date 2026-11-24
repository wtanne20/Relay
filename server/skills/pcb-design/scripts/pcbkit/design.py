"""The circuit-as-code description of a board.

A design file is a Python script that builds a `Design`:

    from pcbkit import Design
    d = Design("blinky", size=(40, 30))
    r1 = d.part("R1", "Device:R", "Resistor_SMD:R_0603_1608Metric", "1k",
                lcsc="C21190", at=(10, 10))
    d.connect("VCC", r1[1], ...)

Coordinates are millimetres from the board's top-left corner, x to the
right and y down (KiCad's convention). Rotation is degrees counter-clockwise.
"""

from .libs import Libraries, split_id

# JLCPCB's standard 2-layer process, with a little margin. These go into the
# KiCad project so DRC checks them.
JLCPCB_RULES = {
    "min_track": 0.15,
    "min_clearance": 0.15,
    "min_via_diameter": 0.5,
    "min_via_drill": 0.3,
    "min_hole": 0.3,
    "min_annular": 0.13,
    "hole_to_hole": 0.5,
    "edge_clearance": 0.3,
    "silk_clearance": 0.0,
    "min_silk_width": 0.15,
    "min_silk_height": 0.8,
}


class Pin:
    def __init__(self, part, number, name):
        self.part = part
        self.number = number
        self.name = name

    def __repr__(self):
        return "%s.%s(%s)" % (self.part.ref, self.number, self.name)


class PinGroup(list):
    """All pins that share a name, e.g. every GND pin of a module."""


class Part:
    def __init__(self, design, ref, symbol, footprint, value, **kw):
        self.design = design
        self.ref = ref
        self.symbol = symbol
        self.footprint = footprint
        self.value = value
        self.lcsc = kw.pop("lcsc", "") or ""
        self.mpn = kw.pop("mpn", "") or ""
        self.at = tuple(kw.pop("at", (0, 0)))
        self.rot = float(kw.pop("rot", 0))
        self.side = kw.pop("side", "top")
        self.dnp = bool(kw.pop("dnp", False))
        self.fields = dict(kw.pop("fields", {}) or {})
        # Extra degrees JLCPCB's assembly needs for this footprint, if their
        # preview shows it rotated. Only affects the CPL file.
        self.jlc_rotation = float(kw.pop("jlc_rotation", 0))
        self.note = kw.pop("note", "")
        self.assemble = kw.pop("assemble", True)
        # Remove the footprint's through-hole pads that are smaller than the
        # fab's minimum drill (e.g. thermal vias under some RF modules).
        self.drop_small_vias = bool(kw.pop("drop_small_vias", False))
        if kw:
            raise TypeError("unknown part option(s) for %s: %s" % (ref, ", ".join(kw)))
        if self.side not in ("top", "bottom"):
            raise ValueError("%s: side must be 'top' or 'bottom'" % ref)
        self._pins = None

    @property
    def pins(self):
        if self._pins is None:
            seen = {}
            for p in self.design.libs.pins(self.symbol):
                seen.setdefault(p["number"], p)
            self._pins = list(seen.values())
        return self._pins

    def pin(self, key):
        key = str(key)
        by_num = [p for p in self.pins if p["number"] == key]
        if by_num:
            return Pin(self, key, by_num[0]["name"])
        raise KeyError("%s has no pin number %r" % (self.ref, key))

    def __getitem__(self, key):
        key = str(key)
        if any(p["number"] == key for p in self.pins):
            return self.pin(key)
        named = [p for p in self.pins if p["name"] == key]
        if named:
            if len(named) == 1:
                return Pin(self, named[0]["number"], key)
            return PinGroup(Pin(self, p["number"], key) for p in named)
        names = sorted({p["name"] for p in self.pins if p["name"] != "~"})
        raise KeyError(
            "%s (%s) has no pin named or numbered %r. Pins: %s" % (self.ref, self.symbol, key, ", ".join(names))
        )

    def __repr__(self):
        return "Part(%s %s)" % (self.ref, self.value)


class Design:
    def __init__(self, name, size, corner_radius=1.0, thickness=1.6, copper_oz=1, origin=(100.0, 100.0)):
        self.name = name
        self.width, self.height = size
        self.corner_radius = corner_radius
        self.thickness = thickness
        self.copper_oz = copper_oz
        # Where the board's top-left corner sits on KiCad's page.
        self.origin = origin
        self.libs = Libraries()
        self.parts = []
        self.nets = {}  # name -> [Pin]
        self.no_connects = []
        self.power_flags = []
        self.netclasses = []
        self.zones = []
        self.keepouts = []
        self.texts = []
        self.review_notes = []
        self.rules = dict(JLCPCB_RULES)
        self.default_track = 0.25
        self.default_clearance = 0.2
        self.default_via = (0.6, 0.3)
        self._hole_n = 0

    # ---- parts ---------------------------------------------------------

    def part(self, ref, symbol, footprint=None, value=None, **kw):
        if any(p.ref == ref for p in self.parts):
            raise ValueError("duplicate reference %s" % ref)
        if footprint is None:
            footprint = self.libs.symbol_property(symbol, "Footprint")
            if not footprint:
                raise ValueError("%s: symbol %s has no default footprint; pass footprint=" % (ref, symbol))
        if value is None:
            value = split_id(symbol)[1]
        p = Part(self, ref, symbol, footprint, value, **kw)
        self.parts.append(p)
        return p

    def hole(self, at, footprint="MountingHole:MountingHole_3.2mm_M3", ref=None):
        """A mounting hole (no net, not assembled)."""
        self._hole_n += 1
        ref = ref or "H%d" % self._hole_n
        p = Part(self, ref, None, footprint, "MountingHole", at=at, assemble=False)
        p._pins = []
        self.parts.append(p)
        return p

    def get(self, ref):
        for p in self.parts:
            if p.ref == ref:
                return p
        raise KeyError("no part %s" % ref)

    # ---- connections ---------------------------------------------------

    def _resolve(self, item):
        if isinstance(item, Pin):
            return [item]
        if isinstance(item, (list, tuple)):
            out = []
            for x in item:
                out += self._resolve(x)
            return out
        if isinstance(item, str) and "." in item:
            ref, key = item.split(".", 1)
            return self._resolve(self.get(ref)[key])
        raise TypeError("can't connect %r; use part[pin] or 'REF.PIN'" % (item,))

    def connect(self, net, *pins):
        """Join pins onto a named net. Calling again with the same name adds
        more pins to it."""
        lst = self.nets.setdefault(net, [])
        for pin in self._resolve(list(pins)):
            for other_net, other in self.nets.items():
                if any(o.part is pin.part and o.number == pin.number for o in other):
                    if other_net != net:
                        raise ValueError("%r is already on net %s, can't also join %s" % (pin, other_net, net))
                    break
            else:
                lst.append(pin)
        return net

    def nc(self, *pins):
        """Mark pins as intentionally unconnected."""
        self.no_connects += self._resolve(list(pins))

    def nc_rest(self, part):
        """Mark every pin of `part` that isn't on a net as unconnected."""
        used = {(p.part.ref, p.number) for pins in self.nets.values() for p in pins}
        used |= {(p.part.ref, p.number) for p in self.no_connects}
        for p in part.pins:
            if (part.ref, p["number"]) not in used:
                self.no_connects.append(Pin(part, p["number"], p["name"]))

    def power_flag(self, *nets):
        """Say these nets are driven from off the board (a connector), so
        ERC doesn't complain that their power-input pins are undriven."""
        self.power_flags += list(nets)

    # ---- board ---------------------------------------------------------

    def netclass(self, name, nets, track=0.25, clearance=0.2, via=(0.6, 0.3)):
        self.netclasses.append(
            {"name": name, "nets": list(nets), "track": track, "clearance": clearance, "via": tuple(via)}
        )

    def zone(self, net, layers=("F.Cu", "B.Cu"), rect=None, clearance=0.3, min_width=0.25, priority=0,
             thermal_gap=0.5, thermal_width=0.5, plane=False):
        """Copper pour. `rect` is (x0, y0, x1, y1); default is the whole board.

        By default the router still connects every pad of the net with
        tracks and the pour is extra copper (it gets broken up by other
        tracks, so it can't be trusted to connect things). Set plane=True
        only for a pour nothing else can cut through (keep tracks out of it
        with keepout()); the router then treats it as the connection."""
        self.zones.append(
            dict(net=net, layers=list(layers), rect=rect, clearance=clearance, min_width=min_width,
                 priority=priority, thermal_gap=thermal_gap, thermal_width=thermal_width, plane=plane)
        )

    def keepout(self, rect, layers=("F.Cu", "B.Cu"), tracks=True, vias=True, pour=True, footprints=False):
        """No copper in `rect` (x0, y0, x1, y1), e.g. under an antenna."""
        self.keepouts.append(dict(rect=rect, layers=list(layers), tracks=tracks, vias=vias, pour=pour, footprints=footprints))

    def text(self, s, at, layer="F.Silkscreen", size=1.0, rot=0):
        self.texts.append(dict(text=s, at=at, layer=layer, size=size, rot=rot))

    def review(self, note):
        """Something a person should check before ordering. Shows up in the
        report and in REVIEW.md."""
        self.review_notes.append(note)

    # ---- checks --------------------------------------------------------

    def check(self):
        """Design-level checks that don't need KiCad. Returns (errors, warnings)."""
        errors, warnings = [], []
        for part in self.parts:
            if part.symbol is None:
                continue
            try:
                pads = self.libs.footprint_pads(part.footprint)
            except KeyError as e:
                errors.append("%s: %s" % (part.ref, e))
                continue
            sym_nums = {p["number"] for p in part.pins}
            missing = sorted(sym_nums - pads)
            if missing:
                errors.append(
                    "%s: symbol %s has pin(s) %s that footprint %s doesn't have. Wrong footprint or pin mapping."
                    % (part.ref, part.symbol, ", ".join(missing), part.footprint)
                )
            extra = sorted(x for x in pads - sym_nums if x)
            if extra:
                warnings.append(
                    "%s: footprint pad(s) %s have no symbol pin and will be left unconnected."
                    % (part.ref, ", ".join(extra))
                )
        connected = {}
        for net, pins in self.nets.items():
            if len(pins) < 2:
                errors.append("net %s has only one pin (%s)" % (net, pins))
            for p in pins:
                connected[(p.part.ref, p.number)] = net
        for p in self.no_connects:
            if (p.part.ref, p.number) in connected:
                errors.append("%r is marked unconnected but is on net %s" % (p, connected[(p.part.ref, p.number)]))
        ncs = {(p.part.ref, p.number) for p in self.no_connects}
        for part in self.parts:
            for p in part.pins:
                k = (part.ref, p["number"])
                if k not in connected and k not in ncs:
                    errors.append(
                        "%s pin %s (%s) is not connected. Connect it or mark it with d.nc()."
                        % (part.ref, p["number"], p["name"])
                    )
        for net in self.power_flags:
            if net not in self.nets:
                errors.append("power_flag on unknown net %s" % net)
        for nc in self.netclasses:
            for n in nc["nets"]:
                if n not in self.nets:
                    warnings.append("netclass %s lists unknown net %s" % (nc["name"], n))
        for z in self.zones:
            if z["net"] not in self.nets:
                errors.append("zone on unknown net %s" % z["net"])
        for part in self.parts:
            if part.symbol and part.assemble and not part.dnp and not part.lcsc:
                warnings.append("%s (%s) has no LCSC part number, so JLCPCB won't place it." % (part.ref, part.value))
        return errors, warnings
