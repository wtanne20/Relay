"""Fabrication outputs for JLCPCB (also fine for PCBWay): Gerbers and drill
files zipped together, a BOM and a pick-and-place (CPL) file in JLCPCB's
column format, the schematic as PDF, and 3D renders."""

import csv
import os
import shutil
import zipfile

from . import tools

GERBER_LAYERS = "F.Cu,B.Cu,F.Paste,B.Paste,F.Silkscreen,B.Silkscreen,F.Mask,B.Mask,Edge.Cuts"


def _cli(args, what):
    code, out = tools.run([tools.kicad_cli()] + args)
    if code != 0:
        raise RuntimeError("kicad-cli %s failed:\n%s" % (what, out))
    return out


def gerbers(pcb, out_dir, zip_path):
    gdir = os.path.join(out_dir, "gerbers")
    shutil.rmtree(gdir, ignore_errors=True)
    os.makedirs(gdir)
    _cli(["pcb", "export", "gerbers", "--layers", GERBER_LAYERS, "--subtract-soldermask", "--use-drill-file-origin", "-o", gdir + os.sep, pcb], "gerbers")
    _cli(
        ["pcb", "export", "drill", "--format", "excellon", "--excellon-units", "mm", "--excellon-separate-th", "--drill-origin", "plot",
         "--generate-map", "--map-format", "gerberx2", "-o", gdir + os.sep, pcb],
        "drill",
    )
    with zipfile.ZipFile(zip_path, "w", zipfile.ZIP_DEFLATED) as z:
        for f in sorted(os.listdir(gdir)):
            z.write(os.path.join(gdir, f), f)
    return sorted(os.listdir(gdir))


def bom(design, path):
    groups = {}
    for p in design.parts:
        if p.symbol is None or not p.assemble or p.dnp:
            continue
        fp = p.footprint.split(":", 1)[1]
        key = (p.value, fp, p.lcsc)
        groups.setdefault(key, []).append(p.ref)
    with open(path, "w", newline="", encoding="utf-8") as f:
        w = csv.writer(f)
        w.writerow(["Comment", "Designator", "Footprint", "LCSC Part #"])
        for (value, fp, lcsc), refs in sorted(groups.items(), key=lambda kv: kv[1][0]):
            w.writerow([value, ",".join(sorted(refs, key=_refkey)), fp, lcsc])
    return len(groups)


def _refkey(r):
    import re

    m = re.match(r"([A-Za-z#]+)(\d+)", r)
    return (m.group(1), int(m.group(2))) if m else (r, 0)


def cpl(design, pcb, path, tmp_dir):
    raw = os.path.join(tmp_dir, "pos-raw.csv")
    _cli(["pcb", "export", "pos", "--format", "csv", "--units", "mm", "--side", "both", "--exclude-dnp", "--use-drill-file-origin", "-o", raw, pcb], "pos")
    parts = {p.ref: p for p in design.parts}
    rows = []
    with open(raw, newline="", encoding="utf-8") as f:
        for r in csv.DictReader(f):
            part = parts.get(r["Ref"])
            if part is None or not part.assemble or part.dnp or part.symbol is None:
                continue
            rot = (float(r["Rot"]) + part.jlc_rotation) % 360
            rows.append(
                [r["Ref"], "%.4fmm" % float(r["PosX"]), "%.4fmm" % float(r["PosY"]),
                 "Top" if r["Side"].lower().startswith("top") else "Bottom", "%g" % rot]
            )
    rows.sort(key=lambda r: _refkey(r[0]))
    with open(path, "w", newline="", encoding="utf-8") as f:
        w = csv.writer(f)
        w.writerow(["Designator", "Mid X", "Mid Y", "Layer", "Rotation"])
        w.writerows(rows)
    return len(rows)


def schematic_pdf(sch, path):
    _cli(["sch", "export", "pdf", "-o", path, sch], "schematic pdf")


def layout_preview(pcb, sch, out_dir):
    """Quick top-down pictures of the placed board (before routing) and the
    schematic, for the user to approve the layout."""
    os.makedirs(out_dir, exist_ok=True)
    made = []
    png = os.path.join(out_dir, "layout-top.png")
    code, log = tools.run([tools.kicad_cli(), "pcb", "render", "--side", "top", "--width", "1400", "--height", "1000",
                           "--quality", "basic", "--background", "opaque", "-o", png, pcb])
    if code == 0 and os.path.exists(png):
        made.append(png)
    svg = os.path.join(out_dir, "layout-top.svg")
    code, log = tools.run([tools.kicad_cli(), "pcb", "export", "svg", "--layers",
                           "F.Fab,F.Courtyard,F.Silkscreen,F.Cu,Edge.Cuts", "--mode-single", "--fit-page-to-board",
                           "--exclude-drawing-sheet", "-o", svg, pcb])
    if code == 0 and os.path.exists(svg):
        made.append(svg)
    pdf = os.path.join(out_dir, "schematic.pdf")
    code, log = tools.run([tools.kicad_cli(), "sch", "export", "pdf", "-o", pdf, sch])
    if code == 0 and os.path.exists(pdf):
        made.append(pdf)
    return made


def schematic_svg(sch, out_dir):
    """The schematic as SVG (opens in any browser), for showing in chat."""
    os.makedirs(out_dir, exist_ok=True)
    code, log = tools.run([tools.kicad_cli(), "sch", "export", "svg", "--exclude-drawing-sheet", "-o", out_dir, sch])
    return sorted(os.path.join(out_dir, f) for f in os.listdir(out_dir) if f.endswith(".svg")) if code == 0 else []


def models_3d(pcb, out_dir, name):
    """A STEP file for mechanical CAD (enclosures) and a GLB with copper,
    mask and silkscreen for viewers (Relay, web pages, Windows 3D Viewer)."""
    made = []
    step = os.path.join(out_dir, name + ".step")
    code, log = tools.run([tools.kicad_cli(), "pcb", "export", "step", "--force", "--subst-models", "--no-dnp",
                           "-o", step, pcb], timeout=900)
    if code == 0 and os.path.exists(step):
        made.append(step)
    glb = os.path.join(out_dir, name + ".glb")
    code, log = tools.run([tools.kicad_cli(), "pcb", "export", "glb", "--force", "--subst-models", "--no-dnp",
                           "--include-tracks", "--include-pads", "--include-zones", "--include-silkscreen",
                           "--include-soldermask", "-o", glb, pcb], timeout=900)
    if code == 0 and os.path.exists(glb):
        made.append(glb)
    return made


def layer_views(pcb, out_dir):
    """SVG drawings of each side's copper and silkscreen."""
    os.makedirs(out_dir, exist_ok=True)
    made = []
    for name, layers in (
        ("top-copper", "F.Cu,F.Silkscreen,Edge.Cuts"),
        ("bottom-copper", "B.Cu,B.Silkscreen,Edge.Cuts"),
        ("assembly-top", "F.Fab,F.Courtyard,Edge.Cuts"),
    ):
        out = os.path.join(out_dir, name + ".svg")
        args = [tools.kicad_cli(), "pcb", "export", "svg", "--layers", layers, "--mode-single", "--fit-page-to-board",
                "--exclude-drawing-sheet", "-o", out, pcb]
        if name.startswith("bottom"):
            args.insert(-1, "--mirror")
        code, log = tools.run(args)
        if code == 0 and os.path.exists(out):
            made.append(out)
    return made


def renders(pcb, out_dir, size=(1600, 1100)):
    os.makedirs(out_dir, exist_ok=True)
    made = []
    views = [
        ("top", ["--side", "top"]),
        ("bottom", ["--side", "bottom"]),
        ("angle", ["--side", "top", "--rotate", "-40,0,-25", "--perspective", "--zoom", "0.85"]),
    ]
    for name, extra in views:
        out = os.path.join(out_dir, "%s.png" % name)
        code, log = tools.run(
            [tools.kicad_cli(), "pcb", "render", "--width", str(size[0]), "--height", str(size[1]), "--quality", "high",
             "--background", "opaque", "-o", out] + extra + [pcb]
        )
        if code == 0 and os.path.exists(out):
            made.append(out)
    return made
