#!/usr/bin/env python3
"""pcbkit command line. Run it with KiCad's Python:

  Windows:  "C:\\Program Files\\KiCad\\9.0\\bin\\python.exe" pcb.py <command> design.py
  Linux:    python3 pcb.py <command> design.py

Commands (each one runs the steps before it, then stops at the first
problem and says what to fix):

  doctor              check KiCad, Java and Freerouting are installed
  setup               download Freerouting
  check  design.py    design checks, schematic, KiCad ERC
  sizes  design.py    each part's real size, and a board size that fits them
  layout design.py    + place parts (automatically where no position is given)
                        and build the board, no tracks yet
  route  design.py    + autoroute with Freerouting and run KiCad DRC
  fab    design.py    + Gerbers, drill, BOM, CPL, schematic, renders, 3D model,
                        and fab/HANDOVER.md: the message to give the user
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

    from pcbkit import placer

    say("== 3/4 Board layout")
    need_tools()
    try:
        lines = placer.auto_place(design)
    except placer.NoRoom as e:
        raise Stop(str(e))
    say("  placement (mm from the board's top-left corner; outline = the part's real courtyard):")
    for line in lines:
        say("    " + line)
    if any(p.auto_placed for p in design.parts):
        say("  [auto] parts were placed automatically. To fix one where it is, copy its at= and rot= into design.py.")
    board, problems = layout.build(design, paths.net, paths.pcb, os.path.basename(paths.sch))
    problems += layout.placement_report(board, design)
    if problems:
        raise Stop(
            "Placement problems (coordinates are mm from the board's top-left corner):\n"
            + "\n".join("  - " + p for p in problems)
        )
    say("  ok: board %gx%g mm with %d footprints" % (design.width, design.height, len(board.GetFootprints())))
    return board


def layout_review(design, paths):
    """Pictures of the placed board and the schematic, and the message that
    asks the user to approve the layout before routing and the 3D model."""
    from pcbkit import fab

    preview = os.path.join(paths.fab, "preview")
    made = fab.layout_preview(paths.pcb, paths.sch, preview)
    out = [
        "**Layout for review: %s** (%g x %g mm, %d parts)" % (design.name, design.width, design.height,
                                                            len([p for p in design.parts if p.symbol])),
        "",
        "Parts are placed but not wired yet. Please check where the connectors, buttons, LEDs and mounting "
        "holes are, and the board size, before I route it and make the 3D model.",
        "",
    ]
    for f in made:
        label = {"layout-top.png": "Download top view (layout-top.png)",
                 "layout-top.svg": "Download outline drawing (layout-top.svg)",
                 "schematic.pdf": "Download schematic (schematic.pdf)"}.get(os.path.basename(f))
        out.append(_link(f, label))
    out += ["", "Where things are (mm from the top-left corner):", ""]
    from pcbkit import placer

    # What a person cares about: parts placed by hand, and anything they
    # plug into, press or look at.
    user_facing = ("J", "SW", "P", "CN", "BT", "LED", "U")
    fixed = [p for p in design.parts if p.symbol and (not p.auto_placed or p.ref.startswith(user_facing))]

    for p in fixed:
        x0, y0, x1, y1 = placer.board_box(p)
        x0, y0, x1, y1 = (round(v) + 0 for v in (x0, y0, x1, y1))
        out.append("- %s %s: x %d-%d, y %d-%d%s" % (p.ref, p.value, x0, x1, y0, y1,
                                                          ", on the %s edge" % p.edge if p.edge else ""))
    holes = [p for p in design.parts if p.symbol is None]
    if holes:
        out.append("- Mounting holes at " + ", ".join("(%g, %g)" % p.at for p in holes))
    out += ["", "%d smaller parts were placed automatically next to the chips they connect to." % len(
        [p for p in design.parts if p.auto_placed and p not in fixed]),
        "", "Reply **OK** to route it, or tell me what to move, resize or rotate."]
    text = "\n".join(out) + "\n"
    with open(os.path.join(paths.fab, "LAYOUT-REVIEW.md"), "w", encoding="utf-8") as fh:
        fh.write(text)
    return text


def step_route(design, paths, passes):
    from pcbkit import route

    say("== 4/4 Routing and DRC")
    stitch = sorted({z["net"] for z in design.zones if {"F.Cu", "B.Cu"} <= set(z["layers"])})
    left, ntracks = route.autoroute(
        paths.pcb, passes=passes, log=lambda s: say("  " + s), stitch_nets=stitch,
        plane_nets=[z["net"] for z in design.zones if z["plane"]], min_track=design.rules["min_track"],
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
    sch_svgs = fab.schematic_svg(paths.sch, os.path.join(paths.fab, "schematic"))
    say("  schematic drawing: " + (", ".join(sch_svgs) if sch_svgs else "none"))
    review = os.path.join(paths.fab, "REVIEW.md")
    write_review(design, review, drc_warnings)
    say("  review checklist: " + review)
    handover = os.path.join(paths.fab, "HANDOVER.md")
    text = write_handover(design, paths, handover)
    say("", "== Hand-over (also in %s). Give this to the user as your final message:" % handover, "", text)


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


def _link(path, label=None):
    """A Relay download link, relative to the folder the command runs in
    (the project folder)."""
    rel = os.path.relpath(path, os.getcwd()).replace(os.sep, "/")
    return "[%s](relay-download://%s)" % (label or "Download " + os.path.basename(path), rel)


def write_handover(design, paths, path):
    """The final message for the user: what the board is, the schematic,
    the renders and the 3D model as download links, and what to check."""
    f = paths.fab
    n = design.name
    def have(p):
        return os.path.exists(p)
    out = ["**%s** is designed: %g x %g mm, %d parts. ERC and DRC pass and every connection is routed."
           % (n, design.width, design.height, len([p for p in design.parts if p.symbol]))]
    out += ["", "**Schematic**", ""]
    pdf = os.path.join(f, n + "-schematic.pdf")
    if have(pdf):
        out.append(_link(pdf, "Download %s-schematic.pdf" % n))
    sch_dir = os.path.join(f, "schematic")
    if os.path.isdir(sch_dir):
        for svg in sorted(os.listdir(sch_dir)):
            if svg.endswith(".svg"):
                out.append(_link(os.path.join(sch_dir, svg)))
    out += ["", "**Board**", ""]
    for img in ("angle", "top", "bottom"):
        p = os.path.join(f, "renders", img + ".png")
        if have(p):
            out.append(_link(p, "Download %s render (%s.png)" % (img, img)))
    out += ["", "**3D model** (tap View in 3D to spin it)", ""]
    glb = os.path.join(f, n + ".glb")
    if have(glb):
        out.append(_link(glb))
    step = os.path.join(f, n + ".step")
    if have(step):
        out.append(_link(step))
    out += ["", "**To order from JLCPCB**", ""]
    for name in (n + "-gerbers.zip", n + "-bom.csv", n + "-cpl.csv", "REVIEW.md"):
        p = os.path.join(f, name)
        if have(p):
            out.append(_link(p))
    out += ["", "**Check before ordering**", ""]
    notes = design.review_notes or ["Nothing flagged in the design file."]
    out += ["- " + note for note in notes]
    out += [
        "- In JLCPCB's assembly preview, check every part's rotation and polarity before paying.",
        "",
        "These checks prove the board can be made and matches the schematic, not that the circuit works. "
        "Bring it up on a current-limited supply first.",
    ]
    text = "\n".join(out) + "\n"
    with open(path, "w", encoding="utf-8") as fh:
        fh.write(text)
    return text


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
                try:
                    x0, y0, x1, y1 = libs.footprint_box(full)
                    say("%s  (%.1f x %.1f mm)" % (full, x1 - x0, y1 - y0))
                except Exception:
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
    size = ""
    if fp:
        try:
            x0, y0, x1, y1 = libs.footprint_box(fp)
            size = "  (%.1f x %.1f mm)" % (x1 - x0, y1 - y0)
        except Exception:
            pass
    say("%s  default footprint: %s%s" % (lib_id, fp or "(none, pass footprint=)", size))
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
        if cmd == "sizes":
            from pcbkit import placer

            for line in placer.sizes(design):
                say(line)
            return 0
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
        if upto == 1:
            os.makedirs(paths.fab, exist_ok=True)
            text = layout_review(design, paths)
            say("", "== Layout review (also in fab/LAYOUT-REVIEW.md). Send this to the user, then STOP and wait "
                "for their OK before running route or fab:", "", text)
        warnings = []
        if upto >= 2:
            warnings = step_route(design, paths, passes)
        if upto >= 3:
            step_fab(design, paths, warnings)
        say("LAYOUT READY: send the review message above and wait for the user's OK" if upto == 1 else "DONE")
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
