"""Tests for the parts of pcbkit that don't need KiCad installed.

    python -m unittest discover -s server/skills/pcb-design/scripts -p "test_*.py"
"""

import os
import sys
import tempfile
import textwrap
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from pcbkit import sexpr  # noqa: E402
from pcbkit.design import Design  # noqa: E402

SYMBOLS = textwrap.dedent(
    """\
    (kicad_symbol_lib
    \t(version 20241209)
    \t(symbol "R"
    \t\t(property "Reference" "R" (at 0 0 0))
    \t\t(property "Footprint" "" (at 0 0 0))
    \t\t(symbol "R_0_1" (rectangle (start -1 -2.54) (end 1 2.54)))
    \t\t(symbol "R_1_1"
    \t\t\t(pin passive line (at 0 3.81 270) (length 1.27) (name "~") (number "1"))
    \t\t\t(pin passive line (at 0 -3.81 90) (length 1.27) (name "~") (number "2"))
    \t\t)
    \t)
    \t(symbol "REG"
    \t\t(property "Reference" "U" (at 0 0 0))
    \t\t(property "Footprint" "Fake:SOT3" (at 0 0 0))
    \t\t(symbol "REG_1_1"
    \t\t\t(pin power_in line (at -7.62 0 0) (length 2.54) (name "IN") (number "1"))
    \t\t\t(pin power_in line (at 0 -7.62 90) (length 2.54) (name "GND") (number "2"))
    \t\t\t(pin power_out line (at 7.62 0 180) (length 2.54) (name "OUT") (number "3"))
    \t\t)
    \t)
    \t(symbol "REG5"
    \t\t(extends "REG")
    \t\t(property "Reference" "U" (at 0 0 0))
    \t\t(property "Value" "REG5" (at 0 0 0))
    \t)
    )
    """
)


def footprint(pads):
    return "(footprint \"x\"\n" + "".join('\t(pad "%s" smd rect (at 0 0) (size 1 1))\n' % p for p in pads) + ")\n"


class PcbkitTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp()
        with open(os.path.join(self.tmp, "Fake.kicad_sym"), "w") as f:
            f.write(SYMBOLS)
        pretty = os.path.join(self.tmp, "Fake.pretty")
        os.mkdir(pretty)
        for name, pads in (("R0603", ["1", "2"]), ("SOT3", ["1", "2", "3"]), ("SOT2", ["1", "2"])):
            with open(os.path.join(pretty, name + ".kicad_mod"), "w") as f:
                f.write(footprint(pads))
        with open(os.path.join(pretty, "TERM.kicad_mod"), "w") as f:
            # Origin at pin 1, body off to the right, like a screw terminal.
            f.write('(footprint "TERM"\n\t(fp_rect (start -2 -3) (end 8 3) (layer "F.CrtYd"))\n'
                    '\t(pad "1" thru_hole rect (at 0 0) (size 2 2))\n\t(pad "2" thru_hole rect (at 5 0) (size 2 2))\n)\n')
        self.d = Design("t", size=(20, 20))
        self.d.libs.symbol_libs = {"Fake": os.path.join(self.tmp, "Fake.kicad_sym")}
        self.d.libs.footprint_libs = {"Fake": pretty}

    def test_sexpr_round_trip(self):
        node = sexpr.parse('(a "b c" (d 1.5) (e "q\\"x"))')
        self.assertEqual(node[1], "b c")
        self.assertIsInstance(node[1], sexpr.Q)
        self.assertEqual(sexpr.parse(sexpr.dump(node)), node)

    def test_extends_is_flattened(self):
        pins = self.d.libs.pins("Fake:REG5")
        self.assertEqual(sorted(p["name"] for p in pins), ["GND", "IN", "OUT"])
        sym = self.d.libs.symbol("Fake:REG5")
        self.assertEqual(sym[1], "Fake:REG5")
        self.assertIsNotNone(sexpr.find(sym, "symbol"))
        self.assertEqual(str(sexpr.find(sym, "symbol")[1]), "REG5_1_1")

    def test_clean_design_passes(self):
        u = self.d.part("U1", "Fake:REG5")
        r = self.d.part("R1", "Fake:R", "Fake:R0603", "1k")
        self.d.connect("VIN", u["IN"], r[1])
        self.d.connect("GND", u["GND"], r[2])
        self.d.nc(u["OUT"])
        errors, _ = self.d.check()
        self.assertEqual(errors, [])

    def test_catches_mistakes(self):
        u = self.d.part("U1", "Fake:REG", "Fake:SOT2")  # footprint lacks pad 3
        r = self.d.part("R1", "Fake:R", "Fake:R0603", "1k")
        self.d.connect("A", u["IN"], r[1])
        errors, _ = self.d.check()
        text = "\n".join(errors)
        self.assertIn("footprint Fake:SOT2 doesn't have", text)
        self.assertIn("U1 pin 2 (GND) is not connected", text)
        self.assertIn("R1 pin 2", text)
        with self.assertRaises(KeyError):
            u["VCC"]
        with self.assertRaises(ValueError):
            self.d.connect("B", r[1])  # already on net A

    def test_real_part_size_comes_from_the_courtyard(self):
        from pcbkit import placer

        self.assertEqual(self.d.libs.footprint_box("Fake:TERM"), (-2.0, -3.0, 8.0, 3.0))
        j = self.d.part("J1", "Fake:R", "Fake:TERM", "TERM", center=(10, 10), rot=90)
        self.d.part("R1", "Fake:R", "Fake:R0603", "1k", at=(3, 3))
        placer.auto_place(self.d)
        # center= puts the middle of the outline there, wherever the origin is.
        x0, y0, x1, y1 = placer.board_box(j)
        self.assertAlmostEqual((x0 + x1) / 2, 10)
        self.assertAlmostEqual((y0 + y1) / 2, 10)
        self.assertAlmostEqual(x1 - x0, 6)  # rotated: 6 wide, 10 tall
        self.assertIn("origin is NOT the middle", "\n".join(placer.sizes(self.d)))

    def test_auto_place_sizes_the_board_and_keeps_parts_apart(self):
        from pcbkit import placer

        d = Design("auto", spacing=1.0)
        d.libs = self.d.libs
        j = d.part("J1", "Fake:R", "Fake:TERM", "TERM", edge="left", rot=90)
        u = d.part("U1", "Fake:REG5", "Fake:SOT3")
        caps = [d.part("C%d" % i, "Fake:R", "Fake:R0603", "1u") for i in range(1, 7)]
        d.connect("VIN", j[1], u["IN"], caps[0][1])
        d.connect("GND", j[2], u["GND"], *[c[2] for c in caps])
        d.connect("OUT", u["OUT"], *[c[1] for c in caps[1:]])
        placer.auto_place(d)
        self.assertGreaterEqual(d.width, 20)
        boxes = [placer.board_box(p) for p in d.parts]
        for b in boxes:
            self.assertGreaterEqual(b[0], -1e-6)
            self.assertGreaterEqual(b[1], -1e-6)
            self.assertLessEqual(b[2], d.width + 1e-6)
            self.assertLessEqual(b[3], d.height + 1e-6)
        for i, a in enumerate(boxes):
            for b in boxes[i + 1:]:
                self.assertFalse(placer._overlaps(a, b, 0.99), (a, b))
        self.assertAlmostEqual(placer.board_box(j)[0], 0)  # flush with the left edge
        # The input cap lands next to the regulator, not across the board.
        ux, uy = u.at
        cx, cy = caps[0].at
        self.assertLess(((ux - cx) ** 2 + (uy - cy) ** 2) ** 0.5, 6)


if __name__ == "__main__":
    unittest.main()
