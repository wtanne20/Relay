"""Runs KiCad's ERC and DRC and turns the JSON reports into short, plain
summaries an agent can act on."""

import json
import os
from collections import OrderedDict

from . import tools
from .sexpr import read_text

# Warnings about how KiCad's library tables are set up on this machine, not
# about the circuit or board.
_LIBRARY_SETUP = {"lib_symbol_issues", "lib_symbol_mismatch", "lib_footprint_issues", "lib_footprint_mismatch"}


def _fmt_items(items, origin=None):
    out = []
    for it in items:
        d = it.get("description", "")
        pos = it.get("pos")
        if pos and origin:
            # KiCad's JSON positions are in mm (page coordinates).
            d += " @ (%.1f, %.1f) mm from board corner" % (pos["x"] - origin[0], pos["y"] - origin[1])
        out.append(d)
    return "; ".join(out)


def _group(violations, origin=None, limit=8):
    groups = OrderedDict()
    for v in violations:
        groups.setdefault((v.get("severity", "error"), v.get("type", "?"), v.get("description", "")), []).append(v)
    lines = []
    for (sev, typ, desc), vs in groups.items():
        lines.append("%s %s x%d: %s" % (sev.upper(), typ, len(vs), desc))
        for v in vs[:limit]:
            lines.append("    - " + _fmt_items(v.get("items", []), origin))
        if len(vs) > limit:
            lines.append("    - ... %d more" % (len(vs) - limit))
    return lines


def erc(sch_path, report_path):
    code, out = tools.run(
        [tools.kicad_cli(), "sch", "erc", "--format", "json", "--severity-all", "--units", "mm", "-o", report_path, sch_path]
    )
    if not os.path.exists(report_path):
        raise RuntimeError("kicad-cli sch erc failed:\n" + out)
    data = json.loads(read_text(report_path))
    vs = []
    for sheet in data.get("sheets", []):
        vs += sheet.get("violations", [])
    vs = [v for v in vs if v.get("type") not in _LIBRARY_SETUP]
    errors = [v for v in vs if v.get("severity") == "error"]
    return errors, vs, _group(vs)


def drc(pcb_path, report_path, origin):
    code, out = tools.run(
        [
            tools.kicad_cli(), "pcb", "drc", "--format", "json", "--severity-all", "--units", "mm",
            "--schematic-parity", "-o", report_path, pcb_path,
        ]
    )
    if not os.path.exists(report_path):
        raise RuntimeError("kicad-cli pcb drc failed:\n" + out)
    data = json.loads(read_text(report_path))
    vs = [v for v in data.get("violations", []) if v.get("type") not in _LIBRARY_SETUP]
    unconnected = data.get("unconnected_items", [])
    parity = data.get("schematic_parity", [])
    lines = []
    if unconnected:
        lines.append("ERROR unconnected x%d: connections the router didn't finish" % len(unconnected))
        for v in unconnected[:10]:
            lines.append("    - " + _fmt_items(v.get("items", []), origin))
    if parity:
        lines.append("ERROR schematic_parity x%d: board doesn't match the schematic" % len(parity))
        for v in parity[:10]:
            lines.append("    - %s: %s" % (v.get("description", ""), _fmt_items(v.get("items", []))))
    lines += _group(vs, origin)
    errors = [v for v in vs if v.get("severity") == "error"] + unconnected + parity
    return errors, vs, lines


def compare_netlist(design, netlist):
    """Makes sure KiCad's netlist (read from the generated schematic) has
    exactly the connections the design asked for."""
    problems = []
    got = {}
    for name, nodes in netlist["nets"].items():
        key = name[1:] if name.startswith("/") else name
        got[key] = {(r, p) for r, p in nodes if not r.startswith("#")}
    for name, pins in design.nets.items():
        want = {(p.part.ref, p.number) for p in pins}
        have = got.get(name)
        if have is None:
            problems.append("net %s is missing from KiCad's netlist" % name)
        elif have != want:
            problems.append(
                "net %s differs: design has %s, schematic has %s"
                % (name, sorted(want - have) or "-", sorted(have - want) or "-")
            )
    return problems
