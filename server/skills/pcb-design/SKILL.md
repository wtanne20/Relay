---
name: pcb-design
description: Design a printed circuit board from a description (schematic, layout, autorouting, ERC/DRC checks) and produce files to order it from JLCPCB or PCBWay, plus renders and a 3D model. Use when the user asks for a PCB, circuit board, schematic, Gerbers or a custom electronics board.
---

# PCB design

You describe the circuit as a Python **design file**. The scripts in this skill turn it into a KiCad 9
schematic and board, place the parts by their real sizes, autoroute the board with Freerouting, run
KiCad's electrical (ERC) and design rule (DRC) checks, and export everything needed to order it. You
never draw anything by hand. You edit the design file and re-run until the checks pass.

`SKILL_DIR` means the folder this SKILL.md is in. **Run every command from the project folder**, because
the download links in the final message are made relative to it.

`SKILL_DIR/GUIDE.md` is the design guide. It covers requirements, power, part choice, circuit rules
by block, placement, routing, mistakes to avoid, and the final checklist. This file covers the
workflow and the commands. The guide covers the decisions.

## How to run commands

Use **KiCad's Python**, which includes KiCad's `pcbnew` module:

- Windows: `"C:\Program Files\KiCad\9.0\bin\python.exe" "SKILL_DIR\scripts\pcb.py" <command>`
- Linux/macOS: `python3 SKILL_DIR/scripts/pcb.py <command>` (on macOS, KiCad's Python is inside KiCad.app)

Below, this is written as `pcb <command>`.

**Timeouts.** `layout`, `route` and `fab` take from under a minute to 10+ minutes, which is far longer
than run_command's default. Always pass `timeout_seconds`: `900` for `layout` and `1800` for `route` and
`fab`. If one still times out, run it again with `3600`.

**Work straight through.** This is one long task. After each command, read the output and make your
next tool call right away. Don't stop to report progress or say what you'll do next, because the user
sees every step. There are only three places to stop:

1. In step 3, if something that matters is missing from the request.
2. At the **layout check-in** (step 8), which is required on every board.
3. At the hand-over (step 11).

Ask exactly what you need when you stop.

---

## The workflow

Follow these steps in order, every time. Each step says what "done" means. Don't move on until it's done.

### Step 1: Tools

Run `pcb doctor`. If something is missing, run `pcb setup` (this downloads Freerouting) or tell the user
exactly what to install:

- KiCad 9: https://www.kicad.org/download/
- Java 25+ (Temurin): https://adoptium.net/

**Done when** doctor reports everything OK.

### Step 2: Read the guide

Read `SKILL_DIR/GUIDE.md` in full, and also `SKILL_DIR/examples/led-strip-controller/design.py`, a
complete working board.

**Done when** you've read both.

### Step 3: Requirements

Fill in the spec block from GUIDE.md section 2. Ask the user, in one message, only about what's
missing *and* matters for safety or power: load current, input voltage range, and whether the board
powers the load. Pick sensible defaults for everything else and note them.

**Done when** the spec block can be written without guessing anything important.

### Step 4: Architecture

Write the power tree and the current budget as comments at the top of `design.py`. Follow GUIDE.md
section 3, and check the LDO heat and regulator headroom.

**Done when** every rail has a source with at least 1.5× headroom.

### Step 5: Parts

For each part, find the symbol, check its pins, and pick the footprint for the real package:

```
pcb symbols esp32 s3                    # find symbols (all words must match)
pcb pins RF_Module:ESP32-S3-WROOM-1     # pin names and types, default footprint and its real size
pcb footprints sot 23 5                 # find footprints; each result shows its real size in mm
pcb parts AP63205WU                     # JLCPCB catalogue: LCSC number, stock, basic/extended
```

Follow GUIDE.md section 4 for footprints, ratings and derating.

**Done when** every part has a symbol and a footprint that match the real part, plus an LCSC number
or a `note=` saying one is still needed.

### Step 6: Write the design file and pass `pcb check`

Copy the example's structure. Write the parts with **no positions yet**, except the few that must be in
a set place (see step 7). Then write the nets, net classes, pours and review notes. Run `pcb check` and
fix every error.

**Done when** `pcb check` reports `ERC clean`.

### Step 7: Placement

First look at the real sizes:

```
pcb sizes design.py
```

This lists every part's real outline (its courtyard) in mm and a board size that would fit them. It
also flags parts whose origin point isn't their middle. Screw terminals and many connectors have their
origin on pin 1, so `at=(x, y)` doesn't put the middle of the part at x, y.

Then decide placement:

- **Fixed by you.** Place connectors, buttons, LEDs a person must reach, mounting holes, and antenna
  modules yourself. Use `edge="left"`, `"right"`, `"top"` or `"bottom"` to put a part flush against
  that edge, and set `rot=` so it faces out. Use `center=(x, y)` to put the middle of the part's real
  outline at x, y. Use `at=(x, y)` only when you mean the footprint origin.
- **Automatic.** Leave `at=` and `center=` out for everything else: chips, regulators, passives. `pcb
  layout` places them by their real outlines, next to the pins they connect to, at least
  `spacing` mm apart (default 1.0, more for big chips), inside the board and out of keepouts. A
  decoupling cap lands by its chip's power pin.
- **Board size.** Give `size=(w, h)` when the user has a size, or leave it out (`Design("name")`) to
  size the board from the parts. It grows until everything fits. With an automatic size, use
  `d.corner_holes()` for mounting holes, and don't give zones or keepouts fixed coordinates.

Run `pcb layout design.py`. It prints every part's size, position and outline, and marks automatic
positions with `[auto]`. If a part is in a bad spot, fix that one part by giving it `at=`, `center=`
or `edge=`, or move the fixed parts. Then run it again.

**Done when** `layout` reports no overlaps and no parts off the board, and the placement follows
GUIDE.md section 6. Connectors must be at edges, the antenna area clear, and the regulator parts
grouped.

### Step 8: Layout check-in with the user (required)

Routing and the 3D model take the longest, so the user approves the layout before you start them. When
`pcb layout` passes, it saves a top-down picture of the placed board (`fab/preview/layout-top.png`)
and the schematic (`fab/preview/schematic.pdf`). It also prints a **layout review message** with
those links, and saves the same message as `fab/LAYOUT-REVIEW.md`.

- Send that message as your reply, copied exactly so the links stay on their own lines. You may add
  one or two sentences on why you placed the main parts where they are.
- **Then stop and wait for the user's answer.** Don't run `route` or `fab` in the same turn.
- If the user asks for changes, make them in `design.py`, run `pcb layout` again, and send the new
  review message. Repeat until they say it's OK.
- Skip this check-in only if the user has explicitly told you to go straight through without checking
  with them.

**Done when** the user has approved the layout.

### Step 9: Route

Run `pcb route design.py` (timeout 1800).

**Done when** it reports `0 connection(s) left` and `DRC clean`. If it doesn't, follow the fix table
below. For unrouted connections, give the router room by spreading parts or making the board bigger
before anything else.

### Step 10: Outputs

Run `pcb fab design.py` (timeout 1800). Then go through the checklist in GUIDE.md section 11. Anything
you can't tick becomes a `d.review("...")` note. If you added notes, run `pcb fab` again.

**Done when** `fab` prints `DONE`.

### Step 11: Present the result

`pcb fab` ends by printing **the hand-over message** and saves it as `fab/HANDOVER.md`. It contains:

- what the board is
- the **schematic** (PDF and SVG)
- the **renders**
- the **3D model** (the Relay app shows a View in 3D button on the `.glb` link)
- the JLCPCB order files
- every review note

Your final reply is that message. Copy it exactly, including every `[...](relay-download://...)` line
on its own line, because those lines become buttons. You may add up to three sentences at the top
about your key design decisions. Don't remove links, and don't describe the board as tested or
working.

---

## Design file reference

```python
from pcbkit import Design
d = Design("my-board", size=(60, 40), corner_radius=2)   # or Design("my-board") to size automatically
d.corner_holes(inset=3.5)                                # four M3 holes in the corners

J1 = d.part("J1", "Connector:Screw_Terminal_01x02",
            "TerminalBlock_Phoenix:TerminalBlock_Phoenix_MKDS-1,5-2-5.08_1x02_P5.08mm_Horizontal",
            "PWR IN", edge="bottom", assemble=False)       # flush with the bottom edge
U1 = d.part("U1", "RF_Module:ESP32-S3-WROOM-1", value="ESP32-S3-WROOM-1-N8",
            lcsc="C...", edge="top", drop_small_vias=True)  # antenna at the top edge
R1 = d.part("R1", "Device:R", "Resistor_SMD:R_0603_1608Metric", "10k", lcsc="C25804")  # placed automatically

d.connect("+3V3", U1["3V3"], R1[1])     # pins by name ("GND" = every GND pin) or number (1)
d.connect("EN", U1["EN"], R1[2])
d.nc(U1["IO1"])                          # deliberately unconnected
d.nc_rest(U1)                            # every pin of U1 not yet on a net
d.power_flag("VIN", "GND")               # rails fed from a connector, diode or inductor

d.netclass("Power", ["VIN", "+5V"], track=0.8, clearance=0.2, via=(0.8, 0.4))
d.zone("GND", layers=["F.Cu", "B.Cu"])  # ground pour on both sides (stitched automatically)
d.keepout((0, 0, 20, 6))                 # x0, y0, x1, y1: no copper, and no automatic parts
d.hole((3.5, 3.5))                       # one M3 hole at a set position
d.text("MY BOARD v1", at=(30, 36), size=1.5)
d.review("Something the user must check before ordering")
```

Coordinates are millimetres from the board's **top-left** corner, with x to the right and y **down**.
`rot=` is in degrees, counter-clockwise.

| Design option | Meaning |
| --- | --- |
| `size=None` | size the board from the parts (rounded up to 5 mm) |
| `spacing=1.0` | minimum gap between automatically placed parts' outlines, in mm |
| `aspect=1.4` | width ÷ height for an automatic size |

| Part option | Meaning |
| --- | --- |
| *(none)* | placed automatically by `pcb layout` |
| `at=(x, y)` | footprint origin here (often pin 1, not the middle: check `pcb sizes`) |
| `center=(x, y)` | middle of the part's real outline here |
| `edge="left"` / `"right"` / `"top"` / `"bottom"` | flush against that edge; set `rot=` so it faces out |
| `rot=` | rotation; automatic parts with no `rot=` may be turned 90° to fit better |
| `side="bottom"` | part on the bottom side |
| `lcsc=`, `mpn=`, `note=` | ordering info; the note lands in REVIEW.md |
| `assemble=False` | hand-soldered: left out of the assembly files |
| `dnp=True` | do not populate |
| `jlc_rotation=` | extra rotation in JLCPCB's placement file only |
| `drop_small_vias=True` | removes footprint vias smaller than the fab allows |
| `fields={"Name": "value"}` | extra schematic fields |

## Commands

Each build command runs all the earlier steps first, then stops at the first problem with a plain
explanation.

```
pcb check  design.py    # design checks, schematic, ERC
pcb sizes  design.py    # every part's real size, and a board size that fits
pcb layout design.py    # + placement (automatic where no position is given), the board, and the
                        #   layout check-in message with a top-down picture
pcb route  design.py    # + Freerouting, ground stitching, DRC
pcb fab    design.py    # + all outputs and the hand-over message
pcb drc    design.py    # DRC only, after hand edits in KiCad
```

| Message | Fix |
| --- | --- |
| `pin X is not connected` | Connect it, or `d.nc()` it if the datasheet says it can float. |
| `has no pin named` | Use a name from `pcb pins <symbol>`. |
| `Input Power pin not driven` | Add the regulator feeding that rail, or `d.power_flag("NET")` if it comes from a connector, diode or inductor. |
| `No room for X` | Make the board bigger, use `size=None`, lower `spacing=`, or move fixed parts. |
| `courtyards overlap` | Move a fixed part by the amount given, or remove its `at=` so it is placed automatically. |
| `pads off the board` | Move the part in, use `edge=`, or check `pcb sizes` for where its origin is. |
| `unconnected` after routing | Give the router room near that position (spread parts, bigger board), then route again. `--passes 80` gives it more attempts. |
| `clearance` | Move parts apart, or lower that netclass's clearance (JLCPCB minimum 0.127 mm). |

Don't edit files in `build/` by hand, because they're regenerated. The KiCad project in `build/`
opens in KiCad for manual tweaks. After hand edits, run `pcb drc`. Running `layout` or `route` again
discards hand edits.

## Outputs (`fab/`)

| File | Use |
| --- | --- |
| `HANDOVER.md` | your final message to the user (step 11) |
| `LAYOUT-REVIEW.md`, `preview/` | the layout check-in (step 8): top-down picture, outline drawing, schematic |
| `<name>-schematic.pdf`, `schematic/*.svg` | schematic |
| `renders/top.png`, `bottom.png`, `angle.png` | photo-style renders |
| `<name>.glb` | 3D model; the Relay app shows it in 3D from its download link |
| `<name>.step` | 3D model for mechanical CAD (enclosures) |
| `<name>-gerbers.zip` | upload to JLCPCB / PCBWay (Gerbers + drill) |
| `<name>-bom.csv`, `<name>-cpl.csv` | JLCPCB assembly: BOM and pick-and-place |
| `REVIEW.md` | what a person must check before ordering |
| `renders/*.svg` | copper and assembly drawings |
