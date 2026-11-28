"""Builds the .kicad_pcb from KiCad's netlist plus the design's placement,
board outline, net classes, zones and keepouts. Needs KiCad's `pcbnew`
Python module, so run it with KiCad's Python."""

import math

import pcbnew

from . import sexpr
from .sexpr import read_text

MM = pcbnew.FromMM


def _v(x, y):
    return pcbnew.VECTOR2I(MM(x), MM(y))


def read_netlist(path):
    """{'components': {ref: {...}}, 'nets': {name: [(ref, pin)]}}"""
    root = sexpr.parse(read_text(path))
    comps = {}
    for c in sexpr.find_all(sexpr.find(root, "components"), "comp"):
        ref = str(sexpr.find(c, "ref")[1])
        fp = sexpr.find(c, "footprint")
        ts = sexpr.find(c, "tstamps")
        sp = sexpr.find(c, "sheetpath")
        comps[ref] = {
            "value": str(sexpr.find(c, "value")[1]),
            "footprint": str(fp[1]) if fp else "",
            "uuid": str(ts[1]) if ts else "",
            "sheet": str(sexpr.find(sp, "tstamps")[1]) if sp else "/",
        }
    nets = {}
    for n in sexpr.find_all(sexpr.find(root, "nets"), "net"):
        name = str(sexpr.find(n, "name")[1])
        nets[name] = [(str(sexpr.find(x, "ref")[1]), str(sexpr.find(x, "pin")[1])) for x in sexpr.find_all(n, "node")]
    return {"components": comps, "nets": nets}


def _netname(design_name, netlist_nets):
    """Local labels come through KiCad's netlist as '/NAME'."""
    if design_name in netlist_nets:
        return design_name
    if "/" + design_name in netlist_nets:
        return "/" + design_name
    return design_name


def _rect_poly(x0, y0, x1, y1, ox, oy):
    return [(x0 + ox, y0 + oy), (x1 + ox, y0 + oy), (x1 + ox, y1 + oy), (x0 + ox, y1 + oy)]


_LAYER_ALIASES = {"F.SilkS": "F.Silkscreen", "B.SilkS": "B.Silkscreen", "F.Fab": "F.Fab", "B.Fab": "B.Fab"}


def _layer(board, name):
    lid = board.GetLayerID(_LAYER_ALIASES.get(name, name))
    if lid < 0:
        raise ValueError("unknown layer %s" % name)
    return lid


def _lset(board, names):
    ls = pcbnew.LSET()
    for n in names:
        ls.AddLayer(_layer(board, n))
    return ls


def _outline(board, design):
    ox, oy = design.origin
    w, h, r = design.width, design.height, design.corner_radius
    edge = pcbnew.Edge_Cuts
    width = MM(0.1)

    def seg(a, b):
        s = pcbnew.PCB_SHAPE(board)
        s.SetShape(pcbnew.SHAPE_T_SEGMENT)
        s.SetStart(_v(*a))
        s.SetEnd(_v(*b))
        s.SetLayer(edge)
        s.SetWidth(width)
        board.Add(s)

    def arc(c, start, mid, end):
        s = pcbnew.PCB_SHAPE(board)
        s.SetShape(pcbnew.SHAPE_T_ARC)
        s.SetArcGeometry(_v(*start), _v(*mid), _v(*end))
        s.SetLayer(edge)
        s.SetWidth(width)
        board.Add(s)

    x0, y0, x1, y1 = ox, oy, ox + w, oy + h
    if r <= 0:
        for a, b in (((x0, y0), (x1, y0)), ((x1, y0), (x1, y1)), ((x1, y1), (x0, y1)), ((x0, y1), (x0, y0))):
            seg(a, b)
        return
    k = r * (1 - math.sqrt(0.5))
    seg((x0 + r, y0), (x1 - r, y0))
    seg((x1, y0 + r), (x1, y1 - r))
    seg((x1 - r, y1), (x0 + r, y1))
    seg((x0, y1 - r), (x0, y0 + r))
    arc(None, (x1 - r, y0), (x1 - k, y0 + k), (x1, y0 + r))
    arc(None, (x1, y1 - r), (x1 - k, y1 - k), (x1 - r, y1))
    arc(None, (x0 + r, y1), (x0 + k, y1 - k), (x0, y1 - r))
    arc(None, (x0, y0 + r), (x0 + k, y0 + k), (x0 + r, y0))


def _zone(board, design, rect, layers, net=None, rule=None, z=None):
    ox, oy = design.origin
    if rect is None:
        inset = 0.0
        rect = (inset, inset, design.width - inset, design.height - inset)
    zone = pcbnew.ZONE(board)
    zone.SetLayerSet(_lset(board, layers))
    outline = zone.Outline()
    outline.NewOutline()
    for x, y in _rect_poly(*rect, ox, oy):
        outline.Append(MM(x), MM(y))
    if rule:
        zone.SetIsRuleArea(True)
        zone.SetDoNotAllowTracks(rule["tracks"])
        zone.SetDoNotAllowVias(rule["vias"])
        zone.SetDoNotAllowCopperPour(rule["pour"])
        zone.SetDoNotAllowPads(False)
        zone.SetDoNotAllowFootprints(rule["footprints"])
    else:
        zone.SetNet(net)
        zone.SetLocalClearance(MM(z["clearance"]))
        zone.SetMinThickness(MM(z["min_width"]))
        zone.SetThermalReliefGap(MM(z["thermal_gap"]))
        zone.SetThermalReliefSpokeWidth(MM(z["thermal_width"]))
        zone.SetAssignedPriority(z["priority"])
        # Solid to SMD pads (reflow), thermal spokes on through-hole pads so
        # they can still be hand-soldered.
        zone.SetPadConnection(pcbnew.ZONE_CONNECTION_THT_THERMAL)
        zone.SetIslandRemovalMode(pcbnew.ISLAND_REMOVAL_MODE_ALWAYS)
    board.Add(zone)
    return zone


def build(design, netlist_path, pcb_path, sch_file):
    nl = read_netlist(netlist_path)
    board = pcbnew.NewBoard(pcb_path)
    board.SetCopperLayerCount(2)
    ds = board.GetDesignSettings()
    ds.SetBoardThickness(MM(design.thickness))
    # Fab outputs measure from the board's bottom-left corner, so Gerbers,
    # drill and pick-and-place coordinates are all positive and line up.
    ds.SetAuxOrigin(_v(design.origin[0], design.origin[1] + design.height))
    r = design.rules
    ds.m_TrackMinWidth = MM(r["min_track"])
    ds.m_MinClearance = MM(r["min_clearance"])
    ds.m_ViasMinSize = MM(r["min_via_diameter"])
    ds.m_MinThroughDrill = MM(r["min_hole"])
    ds.m_ViasMinAnnularWidth = MM(r["min_annular"])
    ds.m_HoleToHoleMin = MM(r["hole_to_hole"])
    ds.m_CopperEdgeClearance = MM(r["edge_clearance"])
    ds.m_SilkClearance = MM(r["silk_clearance"])
    ds.m_MinSilkTextThickness = MM(r["min_silk_width"])
    ds.m_MinSilkTextHeight = MM(r["min_silk_height"])

    ns = ds.m_NetSettings
    default = ns.GetDefaultNetclass()
    default.SetTrackWidth(MM(design.default_track))
    default.SetClearance(MM(design.default_clearance))
    default.SetViaDiameter(MM(design.default_via[0]))
    default.SetViaDrill(MM(design.default_via[1]))
    for i, nc in enumerate(design.netclasses):
        cls = pcbnew.NETCLASS(nc["name"])
        cls.SetTrackWidth(MM(nc["track"]))
        cls.SetClearance(MM(nc["clearance"]))
        cls.SetViaDiameter(MM(nc["via"][0]))
        cls.SetViaDrill(MM(nc["via"][1]))
        cls.SetPriority(i + 1)
        ns.SetNetclass(nc["name"], cls)
        for n in nc["nets"]:
            ns.SetNetclassPatternAssignment(_netname(n, nl["nets"]), nc["name"])

    nets = {}
    for name in nl["nets"]:
        ni = pcbnew.NETINFO_ITEM(board, name)
        board.Add(ni)
        nets[name] = ni
    pad_net = {}
    for name, nodes in nl["nets"].items():
        for ref, pin in nodes:
            pad_net[(ref, pin)] = nets[name]

    ox, oy = design.origin
    problems = []
    for part in design.parts:
        comp = nl["components"].get(part.ref)
        if part.symbol is not None and comp is None:
            problems.append("%s is in the design but not in KiCad's netlist" % part.ref)
            continue
        fp_id = part.footprint
        lib, name = fp_id.split(":", 1)
        fp = pcbnew.FootprintLoad(design.libs.footprint_libs[lib], name)
        if fp is None:
            problems.append("%s: couldn't load footprint %s" % (part.ref, fp_id))
            continue
        fp.SetFPID(pcbnew.LIB_ID(lib, name))
        fp.SetReference(part.ref)
        fp.SetValue(part.value)
        if comp:
            fp.SetPath(pcbnew.KIID_PATH("/" + comp["uuid"]))
            fp.SetSheetname("/")
            fp.SetSheetfile(sch_file)
        board.Add(fp)
        fp.SetPosition(_v(part.at[0] + ox, part.at[1] + oy))
        fp.SetOrientationDegrees(part.rot)
        if part.side == "bottom":
            fp.Flip(fp.GetPosition(), pcbnew.FLIP_DIRECTION_TOP_BOTTOM)
        if part.symbol is None:
            # Mounting holes and other board-only items.
            fp.SetBoardOnly(True)
            fp.SetExcludedFromBOM(True)
            fp.SetExcludedFromPosFiles(True)
        if part.lcsc:
            fp.SetField("LCSC", part.lcsc)
            f = fp.GetFieldByName("LCSC")
            if f:
                f.SetVisible(False)
        if part.drop_small_vias:
            for pad in list(fp.Pads()):
                if pad.HasHole() and pad.GetDrillSize().x < MM(design.rules["min_hole"]):
                    fp.Remove(pad)
        if part.dnp:
            fp.SetDNP(True)
        for pad in fp.Pads():
            n = pad_net.get((part.ref, pad.GetNumber()))
            if n is not None:
                pad.SetNet(n)

    _outline(board, design)
    for z in design.zones:
        _zone(board, design, z["rect"], z["layers"], net=nets[_netname(z["net"], nl["nets"])], z=z)
    for k in design.keepouts:
        _zone(board, design, k["rect"], k["layers"], rule=k)
    for t in design.texts:
        txt = pcbnew.PCB_TEXT(board)
        txt.SetText(t["text"])
        txt.SetPosition(_v(t["at"][0] + ox, t["at"][1] + oy))
        txt.SetLayer(_layer(board, t["layer"]))
        txt.SetTextSize(pcbnew.VECTOR2I(MM(t["size"]), MM(t["size"])))
        txt.SetTextThickness(MM(max(0.15, t["size"] * 0.15)))
        txt.SetTextAngleDegrees(t["rot"])
        if t["layer"].startswith("B."):
            txt.SetMirrored(True)
        board.Add(txt)

    place_references(board, design)
    board.BuildConnectivity()
    pcbnew.SaveBoard(pcb_path, board)
    return board, problems


def place_references(board, design):
    """Moves each reference designator to the first spot beside its part
    (above, below, right, left) that doesn't overlap pads, other parts or
    other text, and hides it if there is no such spot. Values go to the Fab
    layer only, as KiCad does by default."""
    ox, oy = design.origin
    edge = pcbnew.BOX2I(_v(ox + 0.5, oy + 0.5), pcbnew.VECTOR2I(MM(design.width - 1), MM(design.height - 1)))
    blocked = []
    for fp in board.GetFootprints():
        for pad in fp.Pads():
            bb = pad.GetBoundingBox()
            bb.Inflate(MM(0.15))
            blocked.append(bb)
        bb = _body_box(fp)
        blocked.append(bb)
    for d in board.GetDrawings():
        if d.GetLayer() == pcbnew.F_SilkS:
            blocked.append(d.GetBoundingBox())
    for fp in sorted(board.GetFootprints(), key=lambda f: f.GetReference()):
        ref = fp.Reference()
        if not ref.IsVisible():
            continue
        body = _body_box(fp)
        size = MM(0.8) if fp.GetArea() < MM(4) * MM(4) else MM(1.0)
        ref.SetTextSize(pcbnew.VECTOR2I(size, size))
        ref.SetTextThickness(MM(0.15))
        ref.SetTextAngleDegrees(0)
        ref.SetKeepUpright(True)
        tw = int(len(ref.GetText()) * size * 0.9)
        th = size
        gap = MM(0.4)
        cx, cy = body.GetCenter().x, body.GetCenter().y
        spots = [
            (cx, body.GetTop() - gap - th // 2),
            (cx, body.GetBottom() + gap + th // 2),
            (body.GetRight() + gap + tw // 2, cy),
            (body.GetLeft() - gap - tw // 2, cy),
        ]
        placed = False
        for x, y in spots:
            box = pcbnew.BOX2I(pcbnew.VECTOR2I(int(x - tw // 2), int(y - th // 2)), pcbnew.VECTOR2I(int(tw), int(th)))
            if not edge.Contains(box.GetOrigin()) or not edge.Contains(box.GetEnd()):
                continue
            if any(box.Intersects(b) for b in blocked if b is not body):
                continue
            ref.SetPosition(pcbnew.VECTOR2I(int(x), int(y)))
            blocked.append(box)
            placed = True
            break
        if not placed:
            ref.SetVisible(False)


def _body_box(fp):
    """The part's own outline: courtyard if it has one, else its pads."""
    layer = pcbnew.B_CrtYd if fp.IsFlipped() else pcbnew.F_CrtYd
    cy = fp.GetCourtyard(layer)
    if cy.OutlineCount():
        bb = cy.BBox()
        # Don't let a huge keep-clear courtyard (RF modules) hog the board.
        if bb.GetWidth() > MM(30) or bb.GetHeight() > MM(30):
            bb = fp.GetBoundingBox(False, False)
        return bb
    return fp.GetBoundingBox(False, False)


def placement_report(board, design):
    """Parts with pads off the board, or whose courtyards overlap."""
    ox, oy = design.origin
    out = []
    bx0, by0, bx1, by1 = MM(ox), MM(oy), MM(ox + design.width), MM(oy + design.height)
    boxes = []
    for fp in board.GetFootprints():
        side = "B" if fp.IsFlipped() else "F"
        ref = fp.GetReference()
        pads = list(fp.Pads())
        if pads:
            px0 = min(p.GetBoundingBox().GetLeft() for p in pads)
            py0 = min(p.GetBoundingBox().GetTop() for p in pads)
            px1 = max(p.GetBoundingBox().GetRight() for p in pads)
            py1 = max(p.GetBoundingBox().GetBottom() for p in pads)
            if px0 < bx0 or py0 < by0 or px1 > bx1 or py1 > by1:
                out.append(
                    "%s has pads off the board (pads span x %.1f..%.1f, y %.1f..%.1f mm from the board corner)"
                    % (ref, pcbnew.ToMM(px0 - bx0), pcbnew.ToMM(px1 - bx0), pcbnew.ToMM(py0 - by0), pcbnew.ToMM(py1 - by0))
                )
        cy = fp.GetCourtyard(pcbnew.B_CrtYd if side == "B" else pcbnew.F_CrtYd)
        if cy.OutlineCount():
            boxes.append((ref, side, pcbnew.SHAPE_POLY_SET(cy)))
    for i, a in enumerate(boxes):
        for b in boxes[i + 1 :]:
            if a[1] != b[1]:
                continue
            inter = pcbnew.SHAPE_POLY_SET(a[2])
            inter.BooleanIntersection(b[2])
            area = inter.Area() / 1e12 if inter.OutlineCount() else 0
            if area > 0.01:
                bb = inter.BBox()
                out.append(
                    "%s and %s courtyards overlap (%.1f mm2 around x %.1f, y %.1f): move one at least %.1f mm "
                    "sideways or %.1f mm up/down, or remove its at= so it is placed automatically"
                    % (a[0], b[0], area, pcbnew.ToMM(bb.Centre().x - bx0), pcbnew.ToMM(bb.Centre().y - by0),
                       pcbnew.ToMM(bb.GetWidth()) + 0.5, pcbnew.ToMM(bb.GetHeight()) + 0.5)
                )
    return out
