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


if __name__ == "__main__":
    unittest.main()
