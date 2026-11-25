# PCB design guide

How to take a board from a request to files you can order, and get a board that works the first time.
SKILL.md tells you which commands to run. This guide covers how to make good decisions along the way.
Read it once before starting a board, and come back to the section for whatever stage you're on.

The scripts check that the board can be manufactured and that it matches the schematic. They can't
tell whether the circuit works. Most first-spin failures are design mistakes the checks can't see: a
wrong footprint, a strapping pin pulled the wrong way, a regulator that overheats, a connector rated
for less current than it carries. This guide is mostly about avoiding those.

---

## 0. Ground rules

1. **Copy, don't invent.** For every chip, use the datasheet's typical application circuit and the
   recommended layout. Use the values it gives. If you can't see the datasheet, say so in a
   `d.review()` note instead of guessing.
2. **Every number has a source.** That covers current, voltage rating, trace width and component
   values. It's either from the user, from a datasheet, or from a calculation shown in a comment in
   `design.py`. If you assumed something, record it with `d.review()`.
3. **Margin everywhere.** Run parts at no more than about 70% of their voltage, current and power
   ratings, and run ceramic capacitors at no more than 50% of their voltage rating.
4. **Simple beats clever.** Prefer common parts, larger packages (0603/0805), two layers and generous
   spacing. A board that's 10 mm bigger costs almost nothing more. A board that fails costs a whole
   re-spin.
5. **Work straight through.** Fix, rerun and continue until `pcb fab` prints `DONE`. Stop only to ask
   the user something they must decide.
6. **Be honest at hand-over.** Passing ERC and DRC means the board can be built. It doesn't mean the
   board works. List what you weren't sure of.

---

## 1. The process at a glance

| Stage | You're done when |
| --- | --- |
| 1. Requirements | You can fill in the spec block (section 2) without guessing. |
| 2. Architecture | You have a power tree and a current budget, and every rail's regulator has headroom. |
| 3. Parts | Every part exists in KiCad's libraries, has a footprint matching the real package, and has an LCSC number or a note saying it needs one. |
| 4. Schematic (`pcb check`) | ERC is clean, and every pin is either connected or deliberately marked `nc`. |
| 5. Placement (`pcb layout`) | There are no overlaps, connectors sit at the edges, and the critical parts are placed as section 6 says. |
| 6. Routing (`pcb route`) | There are 0 unrouted connections and DRC is clean. |
| 7. Outputs (`pcb fab`) | It prints `DONE`, and you've looked at the renders. |
| 8. Hand-over | You've given the user the links, the REVIEW items and the bring-up steps. |

If a later stage shows an earlier decision was wrong, go back and fix that decision. Patching around
it later is worse. Typical examples are a board too small to route, or a regulator that runs too hot.

---

## 2. Requirements

Write this block as a docstring at the top of `design.py`, filled in. Ask the user about anything
safety- or power-related that they didn't give you. For minor details, pick a sensible default and
record it with `d.review()`.

```
Purpose:           what the board does, in one sentence
Input power:       source (barrel jack, screw terminal, USB-C, battery), voltage MIN-MAX, max current
Loads:             everything the board powers or switches, with voltage and current for each
Interfaces:        connectors and what plugs into each (wire gauge, cable, pinout expectations)
Main chip:         MCU/module and why
Firmware needs:    which pins do what (LED data, buttons, sensors, UART, I2C...)
Size / mounting:   max board size, hole positions, enclosure limits, height limits
Environment:       indoor/outdoor, temperature, vibration, anything unusual
Assembly:          JLCPCB assembles SMD? Which parts are hand-soldered?
Quantity / budget: prototypes or production
```

Questions that are always worth asking:

- **Load current**, for anything that drives LEDs, motors, heaters or solenoids. Don't guess it. If
  the user doesn't know, design for a stated figure and say what that figure is.
- **Input voltage range**, including the worst case. A "12 V" car or solar system can reach 14.4 V
  or more, and it spikes higher.
- **Whether the board powers the load or only controls it.** Passing high current through the board
  changes connector, trace and fuse choices.

---

## 3. Architecture and power

### 3.1 Draw the power tree first

Before you pick any part, write out every rail: where it comes from, what feeds it, and what it feeds.

```
VIN 5-24 V (J1) --fuse--> reverse-polarity --> VIN_P --> buck 5 V 2 A --> +5V --> LDO 3.3 V --> +3V3
                                          \--> load output (J2), passes VIN straight through
USB VBUS --Schottky--> +5V   (diode-OR so USB and VIN can't back-feed each other)
```

### 3.2 Current budget

For each rail, add up the worst-case current of everything on it, then pick a regulator rated for at
least 1.5× that total. These are typical figures to start from. Check them against the datasheets.

| Load | Typical peak |
| --- | --- |
| ESP32 / ESP32-S3 with Wi-Fi transmitting | 350-500 mA peaks on 3.3 V (budget 500 mA) |
| ESP32-C3 | ~350 mA peaks |
| RP2040 | ~50 mA |
| STM32F0/F1/G0 | 20-50 mA |
| WS2812B LED | ~60 mA per LED at full white (12 V strips: ~20 mA per segment of 3) |
| Status LED | 1-5 mA (2 mA is plenty with modern LEDs) |

### 3.3 Picking regulators

- **LDO:** simple and quiet, but it turns the whole voltage drop into heat:
  `P = (Vin - Vout) × I`.
  - A SOT-23-5 can get rid of only about 0.3-0.4 W, and a SOT-223 about 1 W, before it overheats.
  - Example: 5 V to 3.3 V at 300 mA is 0.51 W, which is too much for SOT-23. Use SOT-223, or
    reduce the drop.
  - Don't use an LDO from 12 V or 24 V down to 3.3 V. Use a buck converter.
  - Check the dropout voltage too. A 3.3 V LDO with 0.4 V dropout needs at least 3.7 V in.
- **Buck converter:** use one for any big drop or any current above a few hundred mA.
  - Copy the datasheet's application circuit exactly: inductor value and saturation current, input
    and output caps, bootstrap cap, and feedback divider. The divider is often built in, as in the
    AP63205.
  - The input voltage rating must exceed the maximum input, including spikes, by at least 20%.
  - At low input a buck drops out, so its output sags below the target. Say so if the input range
    reaches near the output voltage. The LED controller example notes this at 5 V in.
- **Pass-through:** if the load runs at the input voltage, as with LED strips, send the input straight
  to the load connector on wide copper, and only regulate the logic supply.

### 3.4 Input protection, from simplest to best

| Need | Use | Notes |
| --- | --- | --- |
| Reverse polarity, low current (< 1 A) | Series Schottky (SS34, SS54) | Drops 0.3-0.5 V and dissipates `Vf × I`. Simple. |
| Reverse polarity, high current | P-channel MOSFET (or ideal-diode IC) in the + line | Almost no loss. Gate to GND through a resistor, with a 12-15 V Zener gate-source clamp when Vin > Vgs max. |
| Over-current | Fuse or PTC rated for the real load | A PTC for self-reset, a blade or SMD fuse for high current. |
| Spikes, ESD, inductive kick | TVS diode on the input (SMBJ series) | Standoff voltage ≥ max normal input. |
| Hot-plug inrush | Bulk electrolytic + TVS | Ceramics alone across a long cable can ring to 2× the voltage. |

Protection usually covers only the board. If the load is fed straight from the input, it gets no
protection unless the protection sits before that split. Say which way you did it.

### 3.5 Multiple sources (USB plus external power)

- Never connect USB VBUS directly to another supply. Diode-OR them with Schottkys, or use a
  load-switch or ideal-diode IC.
- Make sure USB can't back-feed the high-current load output. Plugging in USB shouldn't try to power a
  5 m LED strip.

---

## 4. Choosing parts

### 4.1 Rules

1. **The part must exist in KiCad's libraries.** Check with `pcb symbols`, `pcb pins` and
   `pcb footprints`. If it doesn't, use an equivalent that does, or ask the user.
2. **The footprint must match the exact package** of the part you order. This is the most common
   fatal mistake. For example, SOT-23-5 vs SOT-23-6, SOIC-8 narrow vs wide, inductor size codes, and
   connector pitch (2.54 vs 2.0 vs 1.25 mm). For anything that isn't a standard passive, record the
   package name from the datasheet or LCSC page in `note=`.
3. **Pin numbering must match too.** Two chips in the same package can have different pinouts, so
   check `pcb pins` against the datasheet pin table. Diodes are a common trap, because the pad
   numbers on the symbol and the footprint have to agree. LED and diode polarity are another.
4. **Prefer JLCPCB "basic" parts** for passives and common chips, since extended parts cost an extra
   setup fee each. Use a few extended parts if they make the design better, but don't use them
   for resistors and capacitors.
5. **Package sizes:**
   - **Default:** 0603 for passives. 0805 when you need more capacitance or voltage, or the part may
     be hand-soldered.
   - **Avoid:** 0402 unless space forces it, BGA always, and QFN when an SOIC or TSSOP version exists.
6. **Prefer modules over bare chips for RF.** An ESP32 module comes with its antenna matched and
   certified; a bare chip doesn't.

### 4.2 Ratings and derating

| Part | Rule |
| --- | --- |
| Ceramic capacitor | Voltage rating ≥ 2× working voltage. X5R/X7R only for anything over 1 nF; never Y5V. Expect a 10 µF 0603 at 5 V to give ~4-5 µF (DC bias), so size up or add one. |
| Electrolytic | Rating ≥ 1.25× max input (24 V input means 35 V parts). Check the ripple current rating on buck inputs. |
| Resistor | Power `I²R` or `V²/R` ≤ 50% of rating (0603 = 0.1 W, 0805 = 0.125 W, 1206 = 0.25 W). |
| Diode | Reverse voltage ≥ 1.5× max; average current ≥ 1.5× load. |
| MOSFET | Vds ≥ 1.5× rail; logic-level gate (Rds(on) specified at your Vgs, e.g. 2.5 V or 4.5 V); check `I² × Rds(on)` heat. |
| Inductor | Saturation current ≥ 1.3× peak switch current (not just the load current). |
| Connector | Current rating per contact ≥ 1.25× the real current; screw terminals are usually rated 8-15 A, JST-XH ~3 A, pin headers ~3 A, USB-C (non-PD) 3 A. |

### 4.3 Things to avoid

- **Parts chosen only because a symbol exists.** Confirm a real, stocked part matches it.
- **Obsolete or single-source parts** where common alternatives exist.
- **Tantalum capacitors** on rails that can surge. They fail short. Use ceramic or polymer instead.
- **"Logic-level" MOSFETs that are only specified at Vgs = 10 V.** A 3.3 V GPIO won't fully turn
  them on.
- **Footprints taken from a similar part.** "Close enough" doesn't work for footprints.

---

## 5. Circuit design by block

### 5.1 Decoupling (every chip)

- One 100 nF ceramic per power pin, placed at that pin. Add one 1-10 µF bulk cap per chip or per
  rail section.
- Regulators need the input and output capacitance from the datasheet, on both sides, and they're
  stability-critical. Some LDOs need a minimum output ESR or capacitance.
- Modules such as the ESP32 need 10 µF plus 100 nF at their supply pin, even though they have
  internal decoupling.

### 5.2 Microcontroller essentials

**Every MCU:**
- **Reset:** a pull-up on reset/EN and a capacitor to GND, so it starts cleanly. For the ESP32 EN pin
  that's 10 kΩ and 1 µF. Add a reset button if there's room.
- **Boot mode:** know the boot or strapping pins, and make sure their resting state selects normal
  boot. Fit a BOOT button where the chip has one, such as ESP32 GPIO0 or RP2040 BOOTSEL.
- **Programming access:** always provide a way in: USB, a UART header (TX, RX, GND, 3V3), or SWD
  (SWDIO, SWCLK, GND, 3V3, NRST).
- **Free pins:** bring a couple of spare GPIOs, plus 3V3 and GND, to a header or test pads.

**ESP32 family. Check the pin table for the exact module.**

| Chip | Strapping pins (avoid, or use with care) | Can't use | Notes |
| --- | --- | --- | --- |
| ESP32 (WROOM-32) | GPIO0, 2, 5, 12, 15 | GPIO6-11 (flash) | GPIO34-39 are input-only, no pull-ups. GPIO12 high at boot picks 1.8 V flash and stops boot. |
| ESP32-S3 | GPIO0, 3, 45, 46 | GPIO26-32 (flash); GPIO33-37 on octal-PSRAM modules (…R8) | Native USB on GPIO19 (D-) / GPIO20 (D+). |
| ESP32-C3 | GPIO2, 8, 9 | GPIO12-17 (flash) | Native USB on GPIO18/19. |

- **Avoid strapping pins for outputs** that something external pulls up or down, or that must stay
  quiet at boot, such as LED strip data or relay drivers.
- **Put a pull-down on any pin driving an external load**, such as a MOSFET gate or LED data line,
  so the load stays off while the MCU boots.
- **Keep the antenna** at a board edge with no copper, parts or ground pour in front of or under the
  antenna area, on either layer. Use a `keepout` there. Keep metal out of that area in the enclosure.

**RP2040:**
- **Support parts:** it needs external QSPI flash (W25Q16 or similar), a 12 MHz crystal with load
  caps, and a 27 Ω series resistor on each USB data line.
- **Supply:** the 1.1 V core regulator is internal but needs its own capacitors.
- **Recommended:** a module, such as an RP2040 module footprint or a Pico, unless the user wants the
  bare chip.

**STM32:**
- **Boot pin:** BOOT0 pulled low, with 10 kΩ, for normal boot.
- **Reset:** a 100 nF capacitor on NRST.
- **Analog supply:** VDDA decoupled.
- **VCAP:** some parts need VCAP capacitors. Check the datasheet.

### 5.3 USB-C (device / sink, USB 2.0)

- **CC pull-downs:** CC1 and CC2 each get their own 5.1 kΩ to GND. Never share one resistor between
  them, or many chargers won't supply power.
- **Data lines:** connect both D+ pins together and both D- pins together, because the
  receptacle has two of each.
- **ESD:** add an array (USBLC6-2SC6 or similar) on D+/D- and VBUS for any user-facing port. Place
  it right at the connector.
- **Power:** VBUS gives 5 V at up to 3 A from a capable source, but only 500 mA can be counted on.
  Feed it into the rail through a diode or load switch if there's another supply.
- **Shield:** connect it to GND directly or through 1 MΩ ∥ 4.7 nF. Either way, say which.

### 5.4 Driving loads

| Load | Circuit |
| --- | --- |
| LED (indicator) | Series resistor `R = (Vsupply - Vf) / I`; 1-2 mA is enough. |
| Addressable LED data (WS2812B etc.) | 3.3 V logic is marginal at 5 V; shift to 5 V with a 74AHCT1G125 / 74HCT245 powered from the strip's 5 V; 33-470 Ω series resistor at the output; pull-down so the line is low at boot. For 12/24 V strips, the data line is still 5 V logic. |
| Low-side switch (LED strip, solenoid, motor) | N-MOSFET, gate resistor 22-100 Ω from GPIO, 10-100 kΩ gate pull-down to GND; flyback diode across inductive loads (solenoid, relay coil, brushed motor). |
| Relay | Transistor or MOSFET driver + flyback diode; never drive a coil from a GPIO. |
| Anything off-board | Series resistor (100-330 Ω) on GPIOs that leave the board; ESD/TVS on connectors a person touches. |

### 5.5 Buses and signals

- **I2C:** one set of pull-ups per bus, 2.2-4.7 kΩ to the bus voltage. Don't put a set on every
  device.
- **UART:** TX goes to RX. Label the header from the board's point of view.
- **Analog inputs:** add an RC filter (for example 1 kΩ and 100 nF) and size any divider so the
  pin's maximum voltage is never exceeded at the maximum input.
- **Crystals:** load caps from the datasheet formula, placed right at the crystal. Keep the traces
  short and don't route anything else between the crystal and the chip.

### 5.6 Design for bring-up

- **Power LED:** add a power LED on the main logic rail.
- **Test points:** add test points or pads for every rail and GND, plus a ground loop or pad big
  enough for a probe clip.
- **Isolation:** consider a 0 Ω resistor or solder jumper between a regulator and its load, so a rail
  can be checked with nothing on it.
- **Silkscreen:** label every connector pin (V+, GND, DATA, TX, RX), input voltage range and polarity,
  button functions, and board name and version.

---

## 6. Placement

Placement decides whether the board can be routed and whether it works. Spend time on it, and place
parts in this order:

1. **Mechanical:** board size, mounting holes (at least 3.5 mm from the edge for M3), connectors at
   the edges facing out, buttons and LEDs where people can reach and see them, and the antenna
   area.
2. **Main chip or module:** roughly central, with its antenna at an edge if it has one.
3. **Power section,** as a compact group near the power input:
   - **Order:** connector, then protection, then bulk cap, then regulator.
   - **Buck converters:** the input cap goes right at VIN/GND, then the switch node and inductor,
     then the output caps. Keep the loop of input cap, switch and ground as small as possible. Don't
     put sensitive signals next to the switch node or the inductor.
4. **Decoupling caps:** each one goes within about 2 mm of its pin, on the same side.
5. **Everything else:** grouped by function, close to where its signals go.

**Rules of thumb:**

- **Spacing:** leave 1.5-2 mm between parts, and more around fine-pitch chips and connectors. A
  board that won't route almost always needs more space, not more passes.
- **Orientation:** keep similar parts in the same direction, such as all resistors at 0° or 90°, and
  polarized parts consistently oriented. This makes assembly and inspection easier.
- **One side:** put all SMD parts on the top side unless space forces otherwise. Assembly on both
  sides costs more.
- **Hot parts:** keep LDOs, power resistors and MOSFETs away from the MCU and from temperature
  sensors, and give them copper area.
- **High current:** make the path from input connector to output connector straight and short. Don't
  route it past the logic.
- **Edges:** keep pads at least 0.5 mm from the board edge, except connectors designed to overhang.
- **Overlaps:** read the placement report. Courtyard overlaps are real collisions, so fix every one.

---

## 7. Routing and copper

The router handles ordinary signals. Your job is the net classes, pours and keepouts that make
routing come out right.

### 7.1 Trace width for current

These figures are for an outer layer on 1 oz copper (IPC-2221, about 10 °C rise). For long runs or a
hot enclosure, use the next row down.

| Width | Max current |
| --- | --- |
| 0.25 mm | ~0.8 A |
| 0.5 mm | ~1.4 A |
| 1.0 mm | ~2.4 A |
| 2.0 mm | ~4 A |
| 3.0 mm | ~5.3 A |
| > 5 A | Use a pour (`zone(..., plane=True)` plus a `keepout` so tracks can't cut it), on both layers if possible |

- **Signals:** the default 0.25 mm is fine for signals.
- **Rails:** put power rails in a net class at 0.5-1.0 mm.
- **Vias:** a 0.3 mm drill via carries roughly 1 A. Use several in parallel on power paths, and
  never a single via for a high-current net.

### 7.2 Ground

- **Pour:** use a solid GND pour on both layers (`d.zone("GND", layers=["F.Cu", "B.Cu"])`). The
  scripts add stitching vias.
- **Return paths:** every signal returns through the ground directly under it. Don't let a long
  bottom-layer track cut the ground pour in two under fast signals (USB, SPI, the switch node). Keep
  bottom-layer tracks short, or route them along the edge.
- **One ground:** use a single ground. Don't split it into "analog" and "digital" grounds on a small
  board. Place parts so noisy currents don't flow under sensitive parts.

### 7.3 Specific nets

| Net | Rule |
| --- | --- |
| USB D+/D- | Short, side by side, same length within ~1 mm, no vias if possible, solid ground underneath. |
| Buck switch node | As small as possible: wide but short; no ground-pour slivers or other signals around it. |
| Feedback (regulators) | Taken from the output cap, routed away from the inductor and switch node. |
| Crystal | Short; nothing else routed between crystal and chip; ground underneath. |
| Antenna area | No copper on any layer, enforced with `keepout`. |
| High voltage (> 30 V) | Clearance ≥ 0.5 mm (more for mains, which this workflow should not do). |

### 7.4 When routing fails

Read where the unrouted connections are, then work through these in order:

1. Move parts apart near that spot.
2. Rotate parts so their pins face the parts they connect to.
3. Move a non-critical part to free a channel.
4. Make the board bigger.
5. Try `--passes 80`.

Don't shrink track widths or clearances below what the current needs just to make routing pass.

---

## 8. Design for manufacture (JLCPCB defaults)

The scripts already enforce JLCPCB's two-layer rules: 0.15 mm track and clearance, 0.3 mm via drill,
0.5 mm via diameter, and 0.3 mm board-edge clearance. On top of those rules:

- **Margins:** stay above the minimums where you can. Use 0.2 mm or more clearance and 0.25 mm or
  more tracks for signals. Margins make for better yield and a cheaper board.
- **Silkscreen text:** at least 1.0 mm high and 0.15 mm stroke, never on pads, and readable from one
  or two directions.
- **Polarity:** mark polarity and pin 1 for every diode, LED, electrolytic cap, IC and connector.
  The footprints include this, so don't hide it.
- **Through-hole parts:** these, such as screw terminals and large connectors, are often cheaper to
  hand-solder. Mark them `assemble=False` with a `note=`.
- **Rotation:** JLCPCB's pick-and-place rotation sometimes differs from KiCad's. Always tell the user
  to check the assembly preview before paying. Use `jlc_rotation=` once a part is known to be off.
- **Small vias:** use `drop_small_vias=True` on footprints whose built-in vias are below 0.3 mm
  drill, such as some module thermal vias.
- **Layer count:** these scripts make two-layer boards. If a design truly needs four layers (dense
  BGA, controlled impedance), tell the user instead of forcing it.

---

## 9. Working the tools

- **Timeouts:** run `pcb check` often. It's fast. Give `layout`, `route` and `fab` long timeouts, as
  SKILL.md says.
- **Fixing errors:** read the whole message and fix the cause in `design.py`.
  - Never edit files in `build/`, because they're regenerated.
  - Never mark a pin `nc` just to silence ERC. Mark it `nc` only if the datasheet says it may
    float.
  - Never add a `power_flag` to hide a rail that really has no source.
- **Keep notes:** as you go, add a `d.review()` for every judgement call: assumptions, parts without
  LCSC numbers, unverified pinouts, and current or thermal margins that are tight.
- **Before `pcb fab`,** compare `build/` against this guide's checklist (section 11).
- **After `pcb fab`,** look at the renders:
  - Are connectors at the edges and facing out?
  - Is anything floating or overlapping?
  - Can you read the silkscreen?
  - Is the antenna area clear?

---

## 10. Things to avoid (common first-spin failures)

**Power:**
- An LDO dropping more than ~1.5 V at real current, without a heat calculation.
- Capacitors rated at or just above the working voltage.
- A missing bulk capacitor at a power input that comes in on a cable.
- Two supplies joined without diodes, or USB able to back-feed a load.
- A high-current path routed with default-width tracks or through a single via.
- No fuse on a board that passes several amps to a load.

**Pins and parts:**
- A footprint from "a similar part", or with the wrong pitch, package variant or pin order.
- Diode, LED or electrolytic polarity reversed. Check the symbol and footprint pad numbering.
- Strapping or boot pins used for outputs, or left floating when they set the boot mode.
- Gate or data pins with no pull-down, so the load turns on during boot.
- Inductive loads with no flyback diode.
- USB-C CC pins sharing one resistor, or left unconnected.
- USB D+ and D- swapped.

**Layout:**
- Copper or ground pour under or in front of a module antenna.
- A buck converter's input cap placed far from the chip.
- A feedback trace running past the inductor.
- Parts packed so tightly the router fails, then "fixed" by reducing clearances.
- Mounting holes that collide with copper or parts, or sit too close to the edge.

**Process:**
- Values invented instead of copied from the datasheet.
- Stopping partway to describe the next step instead of doing it.
- Telling the user the board "works" because DRC passed.

---

## 11. Pre-hand-over checklist

Go through this list before you report. Anything you can't tick becomes a `d.review()` note.

- [ ] The spec block at the top of `design.py` is filled in.
- [ ] Every regulator's input voltage range covers the real input, with margin.
- [ ] The current budget fits every regulator at ≤ 70% of its rating, and the LDO power is calculated.
- [ ] The capacitor voltage ratings follow section 4.2.
- [ ] Every connector's current rating covers its load, and high-current paths use the widths in
      section 7.1.
- [ ] Reverse polarity, over-current and ESD are handled, or are listed as not handled.
- [ ] Every IC has its decoupling caps placed at its pins.
- [ ] MCU reset, boot and strapping pins are correct, and programming access exists.
- [ ] Load-driving pins have pull-downs, and inductive loads have flyback diodes.
- [ ] Every footprint was checked against the real part's package, with notes for anything unverified.
- [ ] Every assembled part has an LCSC number, or a note saying it needs one.
- [ ] The silkscreen labels connectors, polarity, voltage, and the board name and version.
- [ ] `pcb fab` printed `DONE`: ERC is clean, DRC is clean, and there are 0 unrouted connections.
- [ ] You've looked at the renders.

---

## 12. Hand-over message

Give the user:

1. **One line** on what the board is and how it's powered.
2. **Download links** on their own lines: the 3D model (`.glb`, which shows a View in 3D button),
   the angled render, the Gerber zip, the BOM, the CPL and the schematic PDF.
3. **The REVIEW items,** most important first. Power and safety items come before cosmetic ones.
4. **Bring-up steps:**
   1. Inspect the board for solder bridges.
   2. Measure the resistance from each rail to GND. Near zero means a short.
   3. Power from a current-limited bench supply, starting low, and check every rail voltage.
   4. Flash a blink test.
   5. Only then connect the real load.
5. **What you couldn't verify:** for example datasheets you couldn't open, or LCSC stock.

Don't describe the board as tested or working. It has passed design checks, and that's all you can
claim.
