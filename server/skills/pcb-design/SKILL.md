---
name: pcb-design
description: Design a printed circuit board from a description (schematic, layout, autorouting, ERC/DRC checks) and produce files to order it from JLCPCB or PCBWay, plus renders and a 3D model. Use when the user asks for a PCB, circuit board, schematic, Gerbers or a custom electronics board.
---

# PCB design

You describe the circuit as a Python **design file**. The scripts in this skill turn it into a KiCad 9
schematic and board, autoroute it with Freerouting, run KiCad's electrical (ERC) and design rule (DRC)
checks, and export everything needed to order the board. You never draw anything by hand: you edit the
design file and re-run until the checks pass.

`SKILL_DIR` below means the folder this SKILL.md is in. Run every command from the project folder.

## 0. Tools

Run commands with **KiCad's Python**, which has KiCad's `pcbnew` module:

- Windows: `"C:\Program Files\KiCad\9.0\bin\python.exe" "SKILL_DIR\scripts\pcb.py" <command>`
- Linux/macOS: `python3 SKILL_DIR/scripts/pcb.py <command>` (KiCad's Python on macOS is inside KiCad.app)

The examples below write this as `pcb <command>`.

First run `pcb doctor`. If something is missing, tell the user exactly what to install:

- **KiCad 9**: https://www.kicad.org/download/ (includes kicad-cli, symbols, footprints and 3D models)
- **Java 25 or newer** for Freerouting: https://adoptium.net/ (Temurin 25)
- **Freerouting**: `pcb setup` downloads it from Maven Central into Relay's `tools` folder.

## 1. Pin down the requirements

Before choosing parts, make sure you know (ask if the request doesn't say):
input voltage range and current, what connects to the board (connectors, wire sizes, currents), the
microcontroller or main chip, board size limits or mounting holes, and whether JLCPCB should assemble the
SMD parts. Don't guess currents for anything that drives a load (LED strips, motors): ask, or design for a
stated figure and say so.

## 2. Choose parts

Use parts that exist in KiCad's libraries, so the symbol and footprint are right:

```
pcb symbols esp32 s3            # find symbols (all words must match)
pcb pins RF_Module:ESP32-S3-WROOM-1   # pin names, pin types, default footprint
pcb footprints sot 23 5         # find footprints
pcb parts AP63205WU             # JLCPCB catalogue: LCSC number, stock, basic/extended (needs internet)
```

- Prefer JLCPCB **basic** parts for passives (no extra assembly fee).
- Check the footprint matches the real part's package. The symbol's default footprint is usually right
  for ICs; for passives, connectors, inductors and switches pick the footprint that matches the LCSC part.
- Check voltage ratings: capacitors on the input rail need a rating above the maximum input voltage.
- Read the datasheet's typical application circuit for every regulator and copy it (input/output caps,
  bootstrap cap, inductor value, feedback). Don't invent values.

## 3. Write the design file

Copy `SKILL_DIR/examples/led-strip-controller/design.py` into the project as a starting point and read it:
it is a complete, working board. The API:

```python
from pcbkit import Design
d = Design("my-board", size=(60, 40), corner_radius=2)          # mm

U1 = d.part("U1", "RF_Module:ESP32-S3-WROOM-1", value="ESP32-S3-WROOM-1-N8",
            lcsc="C...", at=(30, 12), rot=0)                      # footprint defaults to the symbol's
R1 = d.part("R1", "Device:R", "Resistor_SMD:R_0603_1608Metric", "10k", lcsc="C25804", at=(20, 10), rot=90)

d.connect("+3V3", U1["3V3"], R1[1])     # pins by name ("3V3", "GND" = every GND pin) or number (1)
d.connect("EN", U1["EN"], R1[2])
d.nc(U1["IO1"])                          # deliberately unconnected
d.nc_rest(U1)                            # every pin of U1 not yet on a net
d.power_flag("VIN", "GND")               # rails fed from a connector, diode or inductor

d.netclass("Power", ["VIN", "+5V"], track=0.8, clearance=0.2, via=(0.8, 0.4))
d.zone("GND", layers=["F.Cu", "B.Cu"])  # ground pour on both sides (stitched automatically)
d.keepout((0, 0, 20, 6))                 # x0, y0, x1, y1: no copper, e.g. in front of an antenna
d.hole((3.5, 3.5))                       # M3 mounting hole
d.text("MY BOARD v1", at=(30, 36), size=1.5)
d.review("Something the user must check before ordering")
```

Part options: `lcsc=`, `mpn=`, `rot=` (degrees, counter-clockwise), `side="bottom"`, `dnp=True`,
`assemble=False` (hand-soldered, left out of the assembly files), `jlc_rotation=` (extra rotation for
JLCPCB's placement only), `note=`, `drop_small_vias=True` (removes footprint vias smaller than the fab
allows), `fields={"Name": "value"}`.

Coordinates are millimetres from the board's **top-left** corner, x to the right, y **down**.

### Placement rules that matter

- Connectors on the board edge, facing out. Screw terminals and USB connectors overhang the edge slightly.
- Module antennas (ESP32 etc.) at a board edge with nothing in front: no copper, no parts. Their footprint
  includes the keep-clear area.
- Decoupling capacitors within a few mm of the pin they serve. Regulator input cap, switch node and
  inductor close together; keep the switch node small.
- High current paths short and wide (netclass with wide tracks, or a pour with `plane=True` and a
  `keepout` so tracks can't cut it).
- Leave routing room: about 1.5-2 mm between parts, more around fine-pitch chips. If the router can't
  finish, spread parts out or make the board bigger before anything else.

## 4. Build and fix until clean

Each command runs all the earlier steps first and stops at the first problem with a plain explanation:

```
pcb check  design.py    # design checks, schematic, ERC
pcb layout design.py    # + board with parts placed (reports overlaps and parts off the board)
pcb route  design.py    # + Freerouting, ground stitching, DRC
pcb fab    design.py    # + all output files
```

Read the output, change the design file, run the same command again. Typical fixes:

| Message | Fix |
| --- | --- |
| `pin X is not connected` | connect it, or `d.nc()` it if the datasheet says it can float |
| `has no pin named` | use a name from `pcb pins <symbol>` |
| `Input Power pin not driven` | add the regulator feeding that rail, or `d.power_flag("NET")` if it comes from a connector, diode or inductor |
| `courtyards overlap` / `pads off the board` | move parts (positions given in mm from the top-left corner) |
| `unconnected` after routing | give the router room near the listed position, then route again |
| `clearance` | move parts apart, or lower that netclass's clearance (JLCPCB minimum 0.127 mm) |

Don't edit files in `build/` by hand; they are regenerated. `routing` takes 10 seconds to a few minutes.
`--passes 80` gives Freerouting more attempts on a hard board.

## 5. Outputs (`fab/`)

| File | Use |
| --- | --- |
| `<name>-gerbers.zip` | upload to JLCPCB / PCBWay (Gerbers + drill) |
| `<name>-bom.csv`, `<name>-cpl.csv` | JLCPCB assembly: BOM and pick-and-place |
| `<name>-schematic.pdf` | schematic |
| `renders/top.png`, `bottom.png`, `angle.png` | photo-style renders |
| `renders/*.svg` | copper and assembly drawings |
| `<name>.step` | 3D model for mechanical CAD (enclosures) |
| `<name>.glb` | 3D model for viewers |
| `REVIEW.md` | what a person must check before ordering |

The KiCad project in `build/` opens in KiCad for manual tweaks. After hand edits, run `pcb drc design.py`.
Running `layout` or `route` again regenerates the board and discards hand edits.

## 6. Hand over honestly

ERC and DRC passing means the board is manufacturable and matches the schematic. It does **not** mean
the circuit works. When you finish, give the user:

1. The renders and the list of output files.
2. Everything in `REVIEW.md`, especially: footprints you weren't sure matched the real part, pinouts
   you took from the symbol without checking a datasheet, power path choices (current ratings, fusing,
   reverse polarity, voltage ratings), and parts without LCSC numbers.
3. A reminder to check JLCPCB's assembly preview for part rotation before paying.

Add a `d.review(...)` line for every judgement call you made, so it lands in `REVIEW.md`.
