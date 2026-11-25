#!/usr/bin/env python3
"""pcbkit command line. Run it with KiCad's Python:

  Windows:  "C:\\Program Files\\KiCad\\9.0\\bin\\python.exe" pcb.py <command> design.py
  Linux:    python3 pcb.py <command> design.py

Commands (each one runs the steps before it, then stops at the first
problem and says what to fix):

  doctor              check KiCad, Java and Freerouting are installed
  setup               download Freerouting
  check  design.py    design checks, schematic, KiCad ERC
  layout design.py    + build the board with parts placed, no tracks yet
  route  design.py    + autoroute with Freerouting and run KiCad DRC
  fab    design.py    + Gerbers, drill, BOM, CPL, schematic PDF, renders
  drc    design.py    re-run DRC on the current board (after hand edits)
  parts  "query"      search JLCPCB's catalogue (needs internet)
  symbols "query"     find KiCad symbols, e.g. symbols esp32
  footprints "query"  find KiCad footprints, e.g. footprints "SOT-23-5"
  pins Lib:Symbol     list a symbol's pins and its default footprint

Outputs go in build/ and fab/ next to the design file.
"""

import json
import os
import runpy
import sys
import traceback

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)

from pcbkit import tools  # noqa: E402
from pcbkit.design import Design  # noqa: E402
from pcbkit.sexpr import read_text  # noqa: E402


class Stop(Exception):
    """A problem the design author has to fix. Exit code 1."""


def say(*lines):
    for line in lines:
        print(line, flush=True)


def load_design(path):
    path = os.path.abspath(path)
    g = runpy.run_path(path, run_name="pcbkit_design")
    found = [v for v in g.values() if isinstance(v, Design)]
    if not found:
        raise Stop("%s doesn't create a Design. It needs something like: d = Design('name', size=(50, 40))" % path)
    return found[0], os.path.dirname(path)


class Paths:
    def __init__(self, design, folder):
        self.build = os.path.join(folder, "build")
        self.fab = os.path.join(folder, "fab")
        self.reports = os.path.join(self.build, "reports")
        n = design.name
        self.sch = os.path.join(self.build, n + ".kicad_sch")
        self.pro = os.path.join(self.build, n + ".kicad_pro")
        self.pcb = os.path.join(self.build, n + ".kicad_pcb")
        self.net = os.path.join(self.build, n + ".net")
        os.makedirs(self.reports, exist_ok=True)


def need_tools(pcbnew_needed=True):
    if tools.kicad_cli() is None:
        raise Stop("kicad-cli not found. Run: pcb.py doctor")
    if pcbnew_needed:
        try:
            import pcbnew  # noqa: F401
        except ImportError:
            raise Stop("This step needs KiCad's Python (pcbnew). Run pcb.py with KiCad's python.exe. See: pcb.py doctor")


# ---- steps --------------------------------------------------------------


def step_check(design, paths):
    from pcbkit import checks, layout, schematic

    say("== 1/4 Design checks")
    errors, warnings = design.check()
    for w in warnings:
        say("  warning: " + w)
    if errors:
        raise Stop("Design errors:\n" + "\n".join("  - " + e for e in errors))
    say("  ok: %d parts, %d nets" % (len(design.parts), len(design.nets)))

    say("== 2/4 Schematic and ERC")
    need_tools(pcbnew_needed=False)
    prev_uuid = None
    if os.path.exists(paths.sch):
        from pcbkit import sexpr

        try:
            prev_uuid = str(sexpr.find(sexpr.parse(read_text(paths.sch)), "uuid")[1])
        except Exception:
            prev_uuid = None
    text, _, _ = schematic.build(design, design.name, prev_uuid)
    with open(paths.sch, "w", encoding="utf-8") as f:
        f.write(text)
    code, out = tools.run([tools.kicad_cli(), "sch", "export", "netlist", "--format", "kicadsexpr", "-o", paths.net, paths.sch])
    if code != 0:
        raise Stop("KiCad couldn't read the generated schematic:\n" + out)
    diff = checks.compare_netlist(design, layout.read_netlist(paths.net))
    if diff:
        raise Stop("The schematic KiCad read doesn't match the design (pcbkit bug):\n" + "\n".join(diff))
    errs, allv, lines = checks.erc(paths.sch, os.path.join(paths.reports, "erc.json"))
    for line in lines:
        say("  " + line)
    if errs:
        raise Stop(
            "ERC found %d error(s). Fix them in the design file. Common causes: a power input with nothing "
            "driving it (add the regulator, or d.power_flag('NET') if it is fed from a connector, diode or inductor), two outputs "
            "on one net, or a pin that should be d.nc()." % len(errs)
        )
    say("  ok: ERC clean (%d warning(s))" % len(allv))


def step_layout(design, paths):
    from pcbkit import layout

    say("== 3/4 Board layout")
    need_tools()
    board, problems = layout.build(design, paths.net, paths.pcb, os.path.basename(paths.sch))
    problems += layout.placement_report(board, design)
    if problems:
        raise Stop(
            "Placement problems (coordinates are mm from the board's top-left corner):\n"
            + "\n".join("  - " + p for p in problems)
        )
    say("  ok: board %gx%g mm with %d footprints" % (design.width, design.height, len(board.GetFootprints())))


def step_route(design, paths, passes):
    from pcbkit import route

    say("== 4/4 Routing and DRC")
    stitch = sorted({z["net"] for z in design.zones if {"F.Cu", "B.Cu"} <= set(z["layers"])})
    left, ntracks = route.autoroute(
        paths.pcb, passes=passes, log=lambda s: say("  " + s), stitch_nets=stitch,
        plane_nets=[z["net"] for z in design.zones if z["plane"]],
    )
    say("  routed: %d track segments, %d connection(s) left" % (ntracks, left))
    return run_drc(design, paths)


def run_drc(design, paths):
    from pcbkit import checks

    need_tools(pcbnew_needed=False)
    errs, allv, lines = checks.drc(paths.pcb, os.path.join(paths.reports, "drc.json"), design.origin)
    for line in lines:
        say("  " + line)
    if errs:
        raise Stop(
            "DRC found %d error(s). Positions are mm from the board's top-left corner. Usually: move parts "
            "apart or give the router room (bigger board, fewer parts in a corner), then run route again." % len(errs)
        )
    say("  ok: DRC clean (%d warning(s))" % len(allv))
    return allv


def step_fab(design, paths, drc_warnings):
    from pcbkit import fab

    say("== Fabrication files")
    os.makedirs(paths.fab, exist_ok=True)
    n = design.name
    zip_path = os.path.join(paths.fab, n + "-gerbers.zip")
    files = fab.gerbers(paths.pcb, paths.build, zip_path)
    say("  gerbers + drill: %s (%d files)" % (zip_path, len(files)))
    bom = os.path.join(paths.fab, n + "-bom.csv")
    say("  BOM: %s (%d lines)" % (bom, fab.bom(design, bom)))
    cpl = os.path.join(paths.fab, n + "-cpl.csv")
    say("  CPL: %s (%d parts)" % (cpl, fab.cpl(design, paths.pcb, cpl, paths.build)))
    pdf = os.path.join(paths.fab, n + "-schematic.pdf")
    fab.schematic_pdf(paths.sch, pdf)
    say("  schematic: " + pdf)
    imgs = fab.renders(paths.pcb, os.path.join(paths.fab, "renders"))
    say("  renders: " + (", ".join(imgs) if imgs else "none (kicad-cli pcb render failed)"))
    views = fab.layer_views(paths.pcb, os.path.join(paths.fab, "renders"))
    say("  layer drawings: " + (", ".join(views) if views else "none"))
    models = fab.models_3d(paths.pcb, paths.fab, n)
    say("  3D model: " + (", ".join(models) if models else "none (kicad-cli export failed)"))
    review = os.path.join(paths.fab, "REVIEW.md")
    write_review(design, review, drc_warnings)
    say("  review checklist: " + review)


def write_review(design, path, drc_warnings):
    lines = ["# %s: check before ordering" % design.name, ""]
    lines.append("ERC and DRC passed. They can't tell whether the parts, pinouts and power choices are right, so a person should check:")
    lines.append("")
    for note in design.review_notes:
        lines.append("- [ ] " + note)
    lines += [
        "- [ ] Every LCSC number below is the exact part (value, voltage rating, package) and in stock.",
        "- [ ] JLCPCB's assembly preview shows every part the right way round (pin 1, diodes, polarised caps). "
        "Fix any that are off with `jlc_rotation=` in the design file.",
        "",
        "## Parts",
        "",
        "| Ref | Value | Footprint | LCSC | Note |",
        "| --- | --- | --- | --- | --- |",
    ]
    for p in design.parts:
        if p.symbol is None:
            continue
        lines.append("| %s | %s | %s | %s | %s |" % (p.ref, p.value, p.footprint.split(":", 1)[1], p.lcsc or "-", p.note or ""))
    if drc_warnings:
        lines += ["", "## DRC warnings (not blocking)", ""]
        seen = {}
        for v in drc_warnings:
            seen[v.get("description", "")] = seen.get(v.get("description", ""), 0) + 1
        for d, c in seen.items():
            lines.append("- %s (x%d)" % (d, c))
    with open(path, "w", encoding="utf-8") as f:
        f.write("\n".join(lines) + "\n")


# ---- parts search ---------------------------------------------------------


def parts_search(query, limit=15):
    """Searches JLCPCB's parts list through tscircuit's public mirror."""
    import urllib.parse
    import urllib.request

    url = "https://jlcsearch.tscircuit.com/components/list.json?" + urllib.parse.urlencode({"search": query, "limit": limit})
    try:
        with urllib.request.urlopen(url, timeout=30) as r:
            data = json.load(r)
    except Exception as e:
        raise Stop("Couldn't reach the JLCPCB parts search (%s). Check the part on https://jlcpcb.com/parts instead." % e)
    rows = data.get("components", data if isinstance(data, list) else [])
    if not rows:
        say("No parts found for %r" % query)
        return
    for c in rows[:limit]:
        lcsc = c.get("lcsc")
        say(
            "C%s  %-28s %-14s stock %-8s %s %s"
            % (lcsc, str(c.get("mfr", ""))[:28], str(c.get("package", ""))[:14], c.get("stock", "?"),
               "basic" if c.get("is_basic") else "extended", str(c.get("description", ""))[:60])
        )


# ---- library lookups --------------------------------------------------------


def _words(q):
    return [w.lower() for w in q.split() if w]


def find_symbols(query, limit=40):
    import re

    from pcbkit.libs import Libraries

    libs = Libraries()
    words = _words(query)
    n = 0
    for lib in sorted(libs.symbol_libs):
        text = read_text(libs.symbol_libs[lib])
        for m in re.finditer(r'\n\t\(symbol "([^"]+)"', text):
            name = m.group(1)
            full = "%s:%s" % (lib, name)
            if all(w in full.lower() for w in words):
                say(full)
                n += 1
                if n >= limit:
                    say("... more; narrow the search")
                    return
    if n == 0:
        say("No symbols match %r" % query)


def find_footprints(query, limit=40):
    from pcbkit.libs import Libraries

    libs = Libraries()
    words = _words(query)
    n = 0
    for lib in sorted(libs.footprint_libs):
        for f in sorted(os.listdir(libs.footprint_libs[lib])):
            if not f.endswith(".kicad_mod"):
                continue
            full = "%s:%s" % (lib, f[:-10])
            if all(w in full.lower() for w in words):
                say(full)
                n += 1
                if n >= limit:
                    say("... more; narrow the search")
                    return
    if n == 0:
        say("No footprints match %r" % query)


def show_pins(lib_id):
    from pcbkit.libs import Libraries

    libs = Libraries()
    try:
        pins = libs.pins(lib_id)
    except KeyError as e:
        raise Stop(str(e))
    fp = libs.symbol_property(lib_id, "Footprint")
    say("%s  default footprint: %s" % (lib_id, fp or "(none, pass footprint=)"))
    desc = libs.symbol_property(lib_id, "Description")
    if desc:
        say("  " + desc)
    seen = set()
    for p in sorted(pins, key=lambda p: (len(p["number"]), p["number"])):
        if p["number"] in seen:
            continue
        seen.add(p["number"])
        say("  pin %-4s %-14s %s" % (p["number"], p["name"], p["type"]))
    if fp:
        try:
            pads = libs.footprint_pads(fp)
            missing = sorted(seen - pads)
            if missing:
                say("  WARNING: footprint has no pad(s) %s" % ", ".join(missing))
        except KeyError:
            pass


# ---- main -----------------------------------------------------------------


def main(argv):
    if len(argv) < 2 or argv[1] in ("-h", "--help", "help"):
        say(__doc__)
        return 0
    cmd = argv[1]
    passes = 40
    if "--passes" in argv:
        i = argv.index("--passes")
        passes = int(argv[i + 1])
        del argv[i : i + 2]
    try:
        if cmd == "doctor":
            ok, lines = tools.report()
            say(*lines)
            return 0 if ok else 1
        if cmd == "setup":
            if tools.freerouting_jar():
                say("Freerouting already installed: " + tools.freerouting_jar())
            else:
                say("Installed: " + tools.download_freerouting())
            ok, lines = tools.report()
            say(*lines)
            return 0 if ok else 1
        if cmd == "parts":
            parts_search(" ".join(argv[2:]))
            return 0
        if cmd == "symbols":
            find_symbols(" ".join(argv[2:]))
            return 0
        if cmd == "footprints":
            find_footprints(" ".join(argv[2:]))
            return 0
        if cmd == "pins":
            show_pins(argv[2])
            return 0
        if len(argv) < 3:
            raise Stop("usage: pcb.py %s design.py" % cmd)
        design, folder = load_design(argv[2])
        paths = Paths(design, folder)
        if cmd == "drc":
            run_drc(design, paths)
            return 0
        order = ["check", "layout", "route", "fab"]
        if cmd not in order:
            raise Stop("unknown command %r. Try: pcb.py help" % cmd)
        upto = order.index(cmd)
        step_check(design, paths)
        if upto >= 1:
            step_layout(design, paths)
        warnings = []
        if upto >= 2:
            warnings = step_route(design, paths, passes)
        if upto >= 3:
            step_fab(design, paths, warnings)
        say("DONE")
        return 0
    except Stop as e:
        say("", "STOPPED: " + str(e))
        return 1
    except Exception:
        say("", "pcbkit crashed (this is a pcbkit bug, not a design problem):")
        traceback.print_exc()
        return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv))
