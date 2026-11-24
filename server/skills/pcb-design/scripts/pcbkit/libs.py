"""Finds KiCad's symbol and footprint libraries and reads symbols and
footprints from them without needing KiCad's Python module."""

import copy
import glob
import os
import re
import sys

from . import sexpr
from .sexpr import read_text
from .sexpr import Q

_SYMBOL_ENV = ("KICAD9_SYMBOL_DIR", "KICAD8_SYMBOL_DIR", "KICAD_SYMBOL_DIR")
_FOOTPRINT_ENV = ("KICAD9_FOOTPRINT_DIR", "KICAD8_FOOTPRINT_DIR", "KICAD_FOOTPRINT_DIR")


def kicad_share_candidates():
    out = []
    exe_dir = os.path.dirname(os.path.abspath(sys.executable))
    # KiCad's bundled Python on Windows lives in <KiCad>\bin.
    out.append(os.path.join(exe_dir, "..", "share", "kicad"))
    for base in (os.environ.get("ProgramFiles", r"C:\Program Files"), r"C:\Program Files"):
        out += sorted(glob.glob(os.path.join(base, "KiCad", "*", "share", "kicad")), reverse=True)
    out += [
        "/usr/share/kicad",
        "/usr/local/share/kicad",
        "/Applications/KiCad/KiCad.app/Contents/SharedSupport",
    ]
    return [os.path.normpath(p) for p in out]


def _find_dir(env_names, sub):
    for name in env_names:
        v = os.environ.get(name)
        if v and os.path.isdir(v):
            return v
    for share in kicad_share_candidates():
        p = os.path.join(share, sub)
        if os.path.isdir(p):
            return p
    return None


def symbol_dir():
    return _find_dir(_SYMBOL_ENV, "symbols")


def footprint_dir():
    return _find_dir(_FOOTPRINT_ENV, "footprints")


def model_dir():
    return _find_dir(("KICAD9_3DMODEL_DIR", "KICAD8_3DMODEL_DIR", "KICAD_3DMODEL_DIR"), "3dmodels")


class Libraries:
    """Symbol and footprint lookup. Extra libraries (for parts KiCad doesn't
    ship, e.g. imported with easyeda2kicad) can be added per design."""

    def __init__(self):
        self.symbol_libs = {}  # nickname -> .kicad_sym path
        self.footprint_libs = {}  # nickname -> .pretty dir
        self._sym_cache = {}
        sd = symbol_dir()
        if sd:
            for f in glob.glob(os.path.join(sd, "*.kicad_sym")):
                self.symbol_libs[os.path.splitext(os.path.basename(f))[0]] = f
        fd = footprint_dir()
        if fd:
            for d in glob.glob(os.path.join(fd, "*.pretty")):
                self.footprint_libs[os.path.splitext(os.path.basename(d))[0]] = d

    def add_symbol_lib(self, nickname, path):
        self.symbol_libs[nickname] = os.path.abspath(path)

    def add_footprint_lib(self, nickname, path):
        self.footprint_libs[nickname] = os.path.abspath(path)

    # ---- symbols -------------------------------------------------------

    def _raw_symbol(self, lib, name):
        path = self.symbol_libs.get(lib)
        if not path:
            raise KeyError("no symbol library named %r (looked in %s)" % (lib, symbol_dir()))
        key = (path, name)
        if key in self._sym_cache:
            return self._sym_cache[key]
        text = read_text(path)
        m = re.search(r'\n\s*\(symbol "%s"' % re.escape(name), text)
        if not m:
            raise KeyError("symbol %r not found in library %r" % (name, lib))
        start = text.index("(", m.start())
        node = sexpr.parse(text[start : sexpr.block_end(text, start)])
        self._sym_cache[key] = node
        return node

    def symbol(self, lib_id):
        """Returns the symbol as a flattened s-expression named `lib_id`,
        ready to embed in a schematic's lib_symbols."""
        lib, name = split_id(lib_id)
        node = copy.deepcopy(self._raw_symbol(lib, name))
        ext = sexpr.find(node, "extends")
        if ext:
            parent = copy.deepcopy(self._raw_symbol(lib, str(ext[1])))
            pname = str(parent[1])
            child_props = {str(p[1]): p for p in sexpr.find_all(node, "property")}
            flat = [node[0], Q(lib_id)]
            for c in parent[2:]:
                if isinstance(c, list) and c and c[0] == "property" and str(c[1]) in child_props:
                    flat.append(child_props.pop(str(c[1])))
                elif isinstance(c, list) and c and c[0] == "symbol":
                    unit = str(c[1])
                    c[1] = Q(name + unit[len(pname):]) if unit.startswith(pname) else c[1]
                    flat.append(c)
                else:
                    flat.append(c)
            # Child-only properties go after the parent's.
            insert_at = max(i for i, c in enumerate(flat) if isinstance(c, list) and c and c[0] == "property") + 1
            for p in child_props.values():
                flat.insert(insert_at, p)
                insert_at += 1
            node = flat
        else:
            node[1] = Q(lib_id)
        return node

    def pins(self, lib_id):
        """[{number, name, type, x, y, angle, length, unit, hidden}] in
        symbol coordinates (y up)."""
        node = self.symbol(lib_id)
        _, name = split_id(lib_id)
        out = []
        for unit in sexpr.find_all(node, "symbol"):
            m = re.match(r".*_(\d+)_(\d+)$", str(unit[1]))
            u, style = (int(m.group(1)), int(m.group(2))) if m else (1, 1)
            if style > 1:
                continue
            for p in sexpr.find_all(unit, "pin"):
                at = sexpr.find(p, "at")
                ln = sexpr.find(p, "length")
                nm = sexpr.find(p, "name")
                nu = sexpr.find(p, "number")
                out.append(
                    {
                        "number": str(nu[1]),
                        "name": str(nm[1]),
                        "type": str(p[1]),
                        "x": float(at[1]),
                        "y": float(at[2]),
                        "angle": float(at[3]) if len(at) > 3 else 0.0,
                        "length": float(ln[1]) if ln else 2.54,
                        "unit": u,
                        "hidden": "hide" in p or any(
                            isinstance(c, list) and c and c[0] == "hide" and (len(c) < 2 or c[1] == "yes") for c in p
                        ),
                    }
                )
        return out

    def symbol_units(self, lib_id):
        units = sorted({p["unit"] for p in self.pins(lib_id)} - {0})
        return units or [1]

    def symbol_property(self, lib_id, name):
        p = sexpr.prop(self.symbol(lib_id), name)
        return str(p[2]) if p else ""

    def is_power_symbol(self, lib_id):
        return sexpr.find(self.symbol(lib_id), "power") is not None

    def symbol_bbox(self, lib_id, unit=1):
        """Rough bounding box (x0, y0, x1, y1) in symbol coordinates."""
        node = self.symbol(lib_id)
        xs, ys = [], []
        for u in sexpr.find_all(node, "symbol"):
            m = re.match(r".*_(\d+)_(\d+)$", str(u[1]))
            if m and int(m.group(1)) not in (0, unit):
                continue
            for item in u[2:]:
                if not isinstance(item, list):
                    continue
                for key in ("start", "end", "at", "center"):
                    for c in sexpr.find_all(item, key):
                        xs.append(float(c[1]))
                        ys.append(float(c[2]))
                pts = sexpr.find(item, "pts")
                if pts:
                    for xy in sexpr.find_all(pts, "xy"):
                        xs.append(float(xy[1]))
                        ys.append(float(xy[2]))
        if not xs:
            return (-5.08, -5.08, 5.08, 5.08)
        return (min(xs), min(ys), max(xs), max(ys))

    # ---- footprints ----------------------------------------------------

    def footprint_path(self, fp_id):
        lib, name = split_id(fp_id)
        d = self.footprint_libs.get(lib)
        if not d:
            raise KeyError("no footprint library named %r (looked in %s)" % (lib, footprint_dir()))
        p = os.path.join(d, name + ".kicad_mod")
        if not os.path.isfile(p):
            raise KeyError("footprint %r not found in %s" % (name, d))
        return p

    def footprint_pads(self, fp_id):
        """Set of pad numbers in the footprint ('' for unnumbered pads)."""
        node = sexpr.parse(read_text(self.footprint_path(fp_id)))
        pads = set()
        for p in sexpr.find_all(node, "pad"):
            pads.add(str(p[1]))
        return pads

    def footprint_models(self, fp_id):
        node = sexpr.parse(read_text(self.footprint_path(fp_id)))
        return [str(m[1]) for m in sexpr.find_all(node, "model")]


def split_id(lib_id):
    if ":" not in lib_id:
        raise ValueError("expected 'Library:Name', got %r" % lib_id)
    lib, name = lib_id.split(":", 1)
    return lib, name
