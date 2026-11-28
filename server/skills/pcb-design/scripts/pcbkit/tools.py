"""Finding and running the external programs: kicad-cli, Java and
Freerouting."""

import glob
import os
import shutil
import subprocess
import sys
import urllib.request

FREEROUTING_VERSION = "2.5.0"
FREEROUTING_URL = (
    "https://repo1.maven.org/maven2/app/freerouting/freerouting/{v}/freerouting-{v}-executable.jar"
)


def tools_dir():
    """Where downloaded tools live: PCBKIT_TOOLS, else Relay's data folder on
    Windows, else ~/.pcbkit."""
    if os.environ.get("PCBKIT_TOOLS"):
        return os.environ["PCBKIT_TOOLS"]
    local = os.environ.get("LOCALAPPDATA")
    if local:
        return os.path.join(local, "Relay", "tools")
    return os.path.join(os.path.expanduser("~"), ".pcbkit")


def kicad_cli():
    if os.environ.get("KICAD_CLI"):
        return os.environ["KICAD_CLI"]
    exe_dir = os.path.dirname(os.path.abspath(sys.executable))
    for cand in (
        os.path.join(exe_dir, "kicad-cli.exe"),
        os.path.join(exe_dir, "kicad-cli"),
    ):
        if os.path.isfile(cand):
            return cand
    found = shutil.which("kicad-cli")
    if found:
        return found
    base = os.environ.get("ProgramFiles", r"C:\Program Files")
    for cand in sorted(glob.glob(os.path.join(base, "KiCad", "*", "bin", "kicad-cli.exe")), reverse=True):
        return cand
    mac = "/Applications/KiCad/KiCad.app/Contents/MacOS/kicad-cli"
    if os.path.isfile(mac):
        return mac
    return None


def _java_version(java):
    try:
        out = subprocess.run([java, "-version"], capture_output=True, text=True, timeout=30)
    except Exception:
        return None
    text = out.stderr + out.stdout
    import re

    m = re.search(r'version "(\d+)', text)
    return int(m.group(1)) if m else None


def java():
    """A Java 25+ runtime (Freerouting 2.5 needs it)."""
    cands = []
    if os.environ.get("PCBKIT_JAVA"):
        cands.append(os.environ["PCBKIT_JAVA"])
    if os.environ.get("JAVA_HOME"):
        cands.append(os.path.join(os.environ["JAVA_HOME"], "bin", "java"))
    for pattern in (
        r"C:\Program Files\Eclipse Adoptium\jdk-*\bin\java.exe",
        r"C:\Program Files\Eclipse Adoptium\jre-*\bin\java.exe",
        r"C:\Program Files\Java\jdk-*\bin\java.exe",
        r"C:\Program Files\Microsoft\jdk-*\bin\java.exe",
        "/usr/lib/jvm/*/bin/java",
        os.path.join(tools_dir(), "jdk*", "bin", "java*"),
    ):
        cands += sorted(glob.glob(pattern), reverse=True)
    w = shutil.which("java")
    if w:
        cands.append(w)
    best = None
    for c in cands:
        if not os.path.isfile(c):
            continue
        v = _java_version(c)
        if v and v >= 25:
            return c
        if v and (best is None or v > best[1]):
            best = (c, v)
    return None


def freerouting_jar():
    if os.environ.get("FREEROUTING_JAR") and os.path.isfile(os.environ["FREEROUTING_JAR"]):
        return os.environ["FREEROUTING_JAR"]
    jars = sorted(glob.glob(os.path.join(tools_dir(), "freerouting*.jar")), reverse=True)
    return jars[0] if jars else None


def download_freerouting():
    os.makedirs(tools_dir(), exist_ok=True)
    dest = os.path.join(tools_dir(), "freerouting-%s.jar" % FREEROUTING_VERSION)
    url = FREEROUTING_URL.format(v=FREEROUTING_VERSION)
    print("Downloading Freerouting %s from Maven Central..." % FREEROUTING_VERSION)
    tmp = dest + ".part"
    with urllib.request.urlopen(url, timeout=300) as r, open(tmp, "wb") as f:
        shutil.copyfileobj(r, f)
    os.replace(tmp, dest)
    return dest


def run(cmd, cwd=None, timeout=1800):
    p = subprocess.run(cmd, cwd=cwd, capture_output=True, text=True, timeout=timeout)
    return p.returncode, (p.stdout or "") + (p.stderr or "")


def report():
    """Lines describing what's installed and what's missing."""
    lines = []
    ok = True
    try:
        import pcbnew  # noqa: F401

        lines.append("ok   KiCad Python (pcbnew %s): %s" % (pcbnew.Version(), sys.executable))
    except Exception:
        ok = False
        lines.append(
            "MISSING KiCad's Python module. Run this script with KiCad's Python, e.g. "
            r'"C:\Program Files\KiCad\9.0\bin\python.exe". Install KiCad 9 from https://www.kicad.org/download/'
        )
    cli = kicad_cli()
    if cli:
        lines.append("ok   kicad-cli: %s" % cli)
    else:
        ok = False
        lines.append("MISSING kicad-cli (comes with KiCad 9)")
    from . import libs

    for label, d in (("symbols", libs.symbol_dir()), ("footprints", libs.footprint_dir()), ("3D models", libs.model_dir())):
        if d:
            lines.append("ok   KiCad %s: %s" % (label, d))
        else:
            if label != "3D models":
                ok = False
            lines.append("MISSING KiCad %s library folder%s" % (label, " (renders will show bare pads)" if label == "3D models" else ""))
    j = java()
    if j:
        lines.append("ok   Java 25+: %s" % j)
    else:
        ok = False
        lines.append(
            "MISSING Java 25 or newer (Freerouting needs it). Install Temurin 25 from https://adoptium.net/ "
            "or set PCBKIT_JAVA to a java executable."
        )
    jar = freerouting_jar()
    if jar:
        lines.append("ok   Freerouting: %s" % jar)
    else:
        ok = False
        lines.append("MISSING Freerouting. Run: pcb.py setup  (downloads it to %s)" % tools_dir())
    return ok, lines
