"""ESP32-S3 controller for one 3-wire addressable LED strip (WS2812B,
WS2811, WS2815, SK6812...). 5-24 V in; the strip gets the input voltage
straight through, so the strip's voltage must match the supply.

Power:  J1 VIN --+------------------------------------------> J2 V+ (strip)
                 +--D1 (reverse polarity)--U2 buck 5 V--U3 LDO 3.3 V--U1 ESP32-S3
        USB-C VBUS --D2--> 5 V rail (flash and run from USB without VIN)
Data:   U1 GPIO21 --U4 74AHCT1G125 (3.3 V -> 5 V)--R6 33R--> J2 DATA
"""

from pcbkit import Design

d = Design("led-strip-controller", size=(70, 50), corner_radius=2.0)

# ---- parts ----------------------------------------------------------------
# Positions are mm from the board's top-left corner.

R06 = "Resistor_SMD:R_0603_1608Metric"
C06 = "Capacitor_SMD:C_0603_1608Metric"
C08 = "Capacitor_SMD:C_0805_2012Metric"

# Connectors
J1 = d.part("J1", "Connector:Screw_Terminal_01x02",
            "TerminalBlock_Phoenix:TerminalBlock_Phoenix_MKDS-1,5-2-5.08_1x02_P5.08mm_Horizontal",
            "PWR IN 5-24V", at=(5, 44.5), assemble=False,
            note="5.08 mm 2-pin screw terminal, hand-solder")
J2 = d.part("J2", "Connector:Screw_Terminal_01x03",
            "TerminalBlock_Phoenix:TerminalBlock_Phoenix_MKDS-1,5-3-5.08_1x03_P5.08mm_Horizontal",
            "LED OUT", at=(54, 44.5), assemble=False,
            note="5.08 mm 3-pin screw terminal, hand-solder")
J3 = d.part("J3", "Connector:USB_C_Receptacle_USB2.0_16P",
            "Connector_USB:USB_C_Receptacle_HRO_TYPE-C-31-M-12", "USB-C",
            lcsc="C165948", at=(4.2, 20), rot=270)

# ESP32-S3 module. Antenna flush with the top edge, nothing in front of it.
U1 = d.part("U1", "RF_Module:ESP32-S3-WROOM-1", value="ESP32-S3-WROOM-1-N16R8",
            lcsc="C2913202", at=(35, 12.6), drop_small_vias=True,
            note="Footprint's 0.2 mm thermal vias removed: below JLCPCB's 0.3 mm 2-layer minimum")

# Input protection and bulk capacitance
D1 = d.part("D1", "Device:D_Schottky", "Diode_SMD:D_SMA", "SS34", lcsc="C8678", at=(9, 33), rot=90,
            note="40 V 3 A Schottky. Reverse-polarity protection for the controller only, and stops USB power back-feeding the strip")
C1 = d.part("C1", "Device:C_Polarized", "Capacitor_SMD:CP_Elec_6.3x7.7", "100uF 35V", at=(20, 38.6),
            note="Electrolytic, 35 V or more")

# 5 V buck (AP63205: 3.8-32 V in, 2 A)
U2 = d.part("U2", "Regulator_Switching:AP63205WU", value="AP63205WU-7", at=(15, 30.5))
C2 = d.part("C2", "Device:C", "Capacitor_SMD:C_1206_3216Metric", "10uF 50V", at=(15, 26.5),
            note="Buck input cap, X5R/X7R, 50 V")
C3 = d.part("C3", "Device:C", C06, "100nF", lcsc="C14663", at=(19.5, 27.5), note="Bootstrap")
L1 = d.part("L1", "Device:L", "Inductor_SMD:L_APV_ANR5040", "6.8uH", at=(21, 32),
            note="6.8 uH, saturation 3 A or more, 5x5 mm")
C4 = d.part("C4", "Device:C", C08, "22uF", lcsc="C45783", at=(26, 29.5))
C5 = d.part("C5", "Device:C", C08, "22uF", lcsc="C45783", at=(26, 33))

# USB power path and CC resistors
D2 = d.part("D2", "Device:D_Schottky", "Diode_SMD:D_SMA", "SS34", lcsc="C8678", at=(11.5, 13), rot=90)
R1 = d.part("R1", "Device:R", R06, "5.1k", lcsc="C23186", at=(10.5, 21.5), rot=90)
R2 = d.part("R2", "Device:R", R06, "5.1k", lcsc="C23186", at=(10.5, 25), rot=90)

# 3.3 V LDO (AP2112K, 600 mA)
U3 = d.part("U3", "Regulator_Linear:AP2112K-3.3", value="AP2112K-3.3TRG1", lcsc="C51118", at=(32, 31))
C6 = d.part("C6", "Device:C", C08, "10uF", lcsc="C15850", at=(32, 35))
C7 = d.part("C7", "Device:C", C08, "10uF", lcsc="C15850", at=(37, 33))

# ESP32 support
C8 = d.part("C8", "Device:C", C08, "10uF", lcsc="C15850", at=(22, 8.5), rot=90)
C9 = d.part("C9", "Device:C", C06, "100nF", lcsc="C14663", at=(24, 8.5), rot=90)
R3 = d.part("R3", "Device:R", R06, "10k", lcsc="C25804", at=(22, 13), rot=90)
C10 = d.part("C10", "Device:C", C06, "1uF", lcsc="C15849", at=(24, 13), rot=90)
SW1 = d.part("SW1", "Switch:SW_Push", "Button_Switch_SMD:SW_Push_1P1T_XKB_TS-1187A", "RESET",
             lcsc="C318884", at=(17, 9))
SW2 = d.part("SW2", "Switch:SW_Push", "Button_Switch_SMD:SW_Push_1P1T_XKB_TS-1187A", "BOOT",
             lcsc="C318884", at=(53, 20))

# Power LED
D3 = d.part("D3", "Device:LED", "LED_SMD:LED_0603_1608Metric", "GREEN", lcsc="C72043", at=(48, 9))
R4 = d.part("R4", "Device:R", R06, "1k", lcsc="C21190", at=(48, 11.5))

# Data level shifter
U4 = d.part("U4", "74xGxx:74AHCT1G125", "Package_TO_SOT_SMD:SOT-23-5", "SN74AHCT1G125DBVR",
            lcsc="C7484", at=(52, 31))
C11 = d.part("C11", "Device:C", C06, "100nF", lcsc="C14663", at=(52, 27.5))
R5 = d.part("R5", "Device:R", R06, "10k", lcsc="C25804", at=(47, 31), rot=90,
            note="Holds the strip's data line low while the ESP32 boots")
R6 = d.part("R6", "Device:R", R06, "33R", lcsc="C23140", at=(57, 35), rot=90,
            note="Series resistor at the strip's data input")

# Mounting holes (M3)
for xy in ((3.5, 3.5), (66.5, 3.5), (27, 45.5), (43, 45.5)):
    d.hole(xy)

# ---- nets -----------------------------------------------------------------

d.connect("VIN", J1[1], J2[1], D1["A"], C1[1])
d.connect("GND", J1[2], J2[3], C1[2])
d.connect("VIN_P", D1["K"], C2[1], U2["IN"], U2["EN"])
d.connect("SW", U2["SW"], C3[2], L1[1])
d.connect("BST", U2["BST"], C3[1])
d.connect("+5V", L1[2], U2["FB"], C4[1], C5[1], D2["K"], U3["VIN"], U3["EN"], C6[1], U4["VCC"], C11[1])
d.connect("VBUS", J3["VBUS"], D2["A"])
d.connect("CC1", J3["CC1"], R1[1])
d.connect("CC2", J3["CC2"], R2[1])
d.connect("USB_DP", J3["D+"], U1["USB_D+"])
d.connect("USB_DN", J3["D-"], U1["USB_D-"])
d.connect("+3V3", U3["VOUT"], C7[1], U1["3V3"], C8[1], C9[1], R3[1], D3["A"])
d.connect("EN", U1["EN"], R3[2], C10[1], SW1[1])
d.connect("IO0", U1["IO0"], SW2[1])
d.connect("LED_3V3", U1["IO21"], U4[2], R5[1])
d.connect("LED_5V", U4[4], R6[1])
d.connect("LED_DATA", R6[2], J2[2])
d.connect("LED_K", D3["K"], R4[1])
d.connect("GND", C2[2], U2["GND"], C4[2], C5[2], R1[2], R2[2], J3["GND"], J3["SHIELD"],
          U3["GND"], C6[2], C7[2], U1["GND"], C8[2], C9[2], C10[2], SW1[2], SW2[2],
          R4[2], U4["GND"], U4[1], C11[2], R5[2])

d.nc(J3["SBU1"], J3["SBU2"], U3["NC"])
d.nc_rest(U1)
# Rails fed through a diode or inductor (passive pins) need a flag so ERC
# knows they are driven.
d.power_flag("GND", "VIN_P", "+5V")

# ---- board ----------------------------------------------------------------

d.netclass("Power", ["VIN", "VIN_P", "+5V", "SW"], track=0.8, clearance=0.2, via=(0.8, 0.4))
d.netclass("3V3", ["+3V3"], track=0.5, clearance=0.2)

# Strip current runs along the bottom edge: VIN on the bottom layer, GND on
# the top, both solid between the terminals (no tracks allowed there).
d.zone("GND", layers=["F.Cu", "B.Cu"])
d.zone("VIN", layers=["B.Cu"], rect=(0, 40, 70, 50), priority=1, plane=True)
d.keepout((14.5, 42, 49.5, 50), tracks=True, vias=False, pour=False)

d.text("RELAY LED", at=(35, 37.5), size=1.5)
d.text("5-24V", at=(7.5, 38), size=1.0)
d.text("V+ DAT GND", at=(59, 38), size=1.0)

# ---- for a person to check --------------------------------------------------

d.review("Strip current: the VIN/GND path between J1 and J2 is copper pour about 9 mm wide on 1 oz copper, "
         "good for roughly 5 A. Screw terminals of this size are usually rated 8-15 A. Fuse the supply for the strip.")
d.review("Strip power is passed straight through: a 5 V strip needs a 5 V supply, 12 V needs 12 V, 24 V needs 24 V.")
d.review("D1 only protects the controller from reverse polarity. A reversed supply still reaches the strip.")
d.review("The data line is shifted to 5 V by U4 (powered from the 5 V buck). At 5 V input the buck runs in dropout, "
         "so its output is about 4.4 V; fine for WS2812B, but check your strip's data threshold.")
d.review("ESP32 data pin is GPIO21. Set this as the LED pin in WLED/ESPHome.")
d.review("The LCSC numbers in this file were filled in from memory, because JLCPCB's catalogue couldn't be "
         "reached when it was written. Check each one with `pcb parts <number>` or on jlcpcb.com/parts.")
d.review("Choose LCSC numbers for U2 (AP63205WU-7), L1 (6.8 uH, 5x5 mm, 3 A+), C1 (100 uF 35 V electrolytic, "
         "6.3x7.7 mm) and C2 (10 uF 50 V 1206). Make sure each part matches its footprint.")
d.review("ESP32-S3-WROOM-1-N16R8 uses octal PSRAM, so GPIO35-37 are unavailable (they are left unconnected here).")
d.review("Antenna: nothing should sit in front of the module's antenna in the enclosure; keep metal away.")
