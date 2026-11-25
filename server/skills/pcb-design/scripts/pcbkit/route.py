"""Autorouting with Freerouting through KiCad's Specctra DSN/SES export and
import, then filling the copper zones."""

import os
import pcbnew

from . import tools


def strip_tracks(board):
    for t in list(board.GetTracks()):
        board.Remove(t)


def unrouted(board):
    board.BuildConnectivity()
    return board.GetConnectivity().GetUnconnectedCount(True)


def fill_zones(board):
    filler = pcbnew.ZONE_FILLER(board)
    filler.Fill(board.Zones())


def _rule_areas(board):
    areas = [z for z in board.Zones() if z.GetIsRuleArea() and z.GetDoNotAllowVias()]
    for fp in board.GetFootprints():
        areas += [z for z in fp.Zones() if z.GetIsRuleArea() and z.GetDoNotAllowVias()]
    return areas


def stitch(board, net_name, pitch=4.0, via=(0.6, 0.3)):
    """Drops vias joining the top and bottom pours of `net_name` wherever a
    via fits completely inside both fills. Freerouting treats a pour as one
    solid plane, so without these the real pours end up in disconnected
    pieces. Returns how many vias were added."""
    zones = [z for z in board.Zones() if not z.GetIsRuleArea() and z.GetNetname() in (net_name, "/" + net_name)]
    if not zones:
        return 0
    net = zones[0].GetNet()
    layers = (pcbnew.F_Cu, pcbnew.B_Cu)
    if not all(any(z.IsOnLayer(l) for z in zones) for l in layers):
        return 0
    rules = _rule_areas(board)
    holes = [fp.GetPosition() for fp in board.GetFootprints() if not fp.GetPadCount() or all(
        p.GetAttribute() == pcbnew.PAD_ATTRIB_NPTH for p in fp.Pads())]
    holes += [p.GetPosition() for fp in board.GetFootprints() for p in fp.Pads() if p.HasHole()]
    existing = [v.GetPosition() for v in board.GetTracks() if isinstance(v, pcbnew.PCB_VIA)]
    bb = board.GetBoardEdgesBoundingBox()
    r = pcbnew.FromMM(via[0] / 2 + 0.05)
    ring = [(0, 0)] + [(int(r * dx), int(r * dy)) for dx, dy in
                       ((1, 0), (-1, 0), (0, 1), (0, -1), (0.7, 0.7), (-0.7, 0.7), (0.7, -0.7), (-0.7, -0.7))]
    step = pcbnew.FromMM(pitch)
    keep = pcbnew.FromMM(2.5)
    added = 0
    y = bb.GetTop() + step // 2
    while y < bb.GetBottom():
        x = bb.GetLeft() + step // 2
        while x < bb.GetRight():
            pos = pcbnew.VECTOR2I(int(x), int(y))
            ok = all(
                any(z.HitTestFilledArea(l, pcbnew.VECTOR2I(pos.x + dx, pos.y + dy)) for z in zones)
                for l in layers
                for dx, dy in ring
            )
            if ok and any(a.Outline().Contains(pos) for a in rules):
                ok = False
            if ok and any((pos - h).EuclideanNorm() < keep for h in holes + existing):
                ok = False
            if ok:
                v = pcbnew.PCB_VIA(board)
                v.SetPosition(pos)
                v.SetWidth(pcbnew.FromMM(via[0]))
                v.SetDrill(pcbnew.FromMM(via[1]))
                v.SetNet(net)
                board.Add(v)
                existing.append(pos)
                added += 1
            x += step
        y += step
    return added


def autoroute(pcb_path, passes=40, threads=None, log=print, stitch_nets=(), plane_nets=(), min_track=0.15):
    board = pcbnew.LoadBoard(pcb_path)
    strip_tracks(board)
    # Freerouting treats every pour as a solid plane that already connects
    # its pads. Hide the pours that aren't real planes so their nets get
    # routed with tracks. (This copy of the board is never saved.)
    planes = set(plane_nets) | {"/" + n for n in plane_nets}
    for z in list(board.Zones()):
        if not z.GetIsRuleArea() and z.GetNetname() not in planes:
            board.Remove(z)
    base = os.path.splitext(pcb_path)[0]
    dsn, ses = base + ".dsn", base + ".ses"
    for f in (dsn, ses):
        if os.path.exists(f):
            os.remove(f)
    if not pcbnew.ExportSpecctraDSN(board, dsn):
        raise RuntimeError("KiCad couldn't export the board for routing (Specctra DSN)")
    java, jar = tools.java(), tools.freerouting_jar()
    if not java or not jar:
        raise RuntimeError("Freerouting isn't set up. Run: pcb.py doctor")
    cmd = [
        java,
        "-Djava.awt.headless=true",
        "-jar",
        jar,
        "-de",
        dsn,
        "-do",
        ses,
        "-mp",
        str(passes),
        "--gui.enabled=false",
        "--usage_and_diagnostic_data.disable_analytics=true",
        "--api_server.enabled=false",
        "--mcp_server.enabled=false",
    ]
    if threads:
        cmd += ["-mt", str(threads)]
    log("Routing with Freerouting (up to %d passes)..." % passes)
    code, out = tools.run(cmd, cwd=os.path.dirname(os.path.abspath(pcb_path)))
    with open(base + "-freerouting.log", "w", encoding="utf-8") as f:
        f.write(out)
    if not os.path.exists(ses):
        tail = "\n".join(out.strip().splitlines()[-15:])
        raise RuntimeError("Freerouting didn't produce a session file (exit %s):\n%s" % (code, tail))
    board = pcbnew.LoadBoard(pcb_path)
    strip_tracks(board)
    if not pcbnew.ImportSpecctraSES(board, ses):
        raise RuntimeError("KiCad couldn't import Freerouting's result")
    # Freerouting sometimes necks a track down where it enters a small pad,
    # below the fab's minimum width. Those stubs are tiny; bring them up to
    # the minimum (DRC still checks the clearance afterwards).
    floor = pcbnew.FromMM(min_track)
    widened = 0
    for t in board.GetTracks():
        if not isinstance(t, pcbnew.PCB_VIA) and t.GetWidth() < floor:
            t.SetWidth(floor)
            widened += 1
    if widened:
        log("Widened %d track stub(s) to the %.2f mm minimum" % (widened, min_track))
    fill_zones(board)
    for net in stitch_nets:
        n = stitch(board, net)
        if n:
            log("Added %d stitching vias to %s" % (n, net))
    fill_zones(board)
    left = unrouted(board)
    pcbnew.SaveBoard(pcb_path, board)
    return left, len(board.GetTracks())
