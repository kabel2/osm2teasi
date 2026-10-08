"""Update BikeNav/packages.xml on the device after replacing map files.

At startup bikenav.exe checks every file of every map package listed in
packages.xml (FUN_0028937c -> FUN_002891d8 -> FUN_00288b04):
  FUN_002887c0: <size> == file size
  FUN_00288858: <md5>  == MD5(file[0x44:0x444] + file[-0x400:])
Otherwise it shows "error_message_mappackage_tahuna" ("Karten nicht korrekt
installiert ...").  The first KB is read from 0x44, i.e. without salt and MAC, so
the value survives the firmware re-signing a file for the device.

usage: packages.py <packages.xml> <map file> [<map file> ...]
"""
import hashlib
import os
import re
import sys


def package_md5(d):
    return hashlib.md5(d[0x44:0x444] + d[-0x400:]).hexdigest()


def update(xml, name, d):
    """Set <md5>/<size> of the <file> entry whose <url> ends with /name."""
    pat = re.compile(r"(<url>[^<]*/" + re.escape(name) + r"</url>\s*<md5>)[0-9a-f]*(</md5>\s*<size>)\d+(</size>)")
    xml, n = pat.subn(lambda m: m.group(1) + package_md5(d) + m.group(2) + str(len(d)) + m.group(3), xml)
    if n != 1:
        raise KeyError(f"{name}: {n} entries in packages.xml")
    return xml


if __name__ == "__main__":
    if len(sys.argv) < 3:
        sys.exit(__doc__)
    path = sys.argv[1]
    xml = open(path, encoding="utf-8", newline="").read()
    for f in sys.argv[2:]:
        d = open(f, "rb").read()
        xml = update(xml, os.path.basename(f), d)
        print(os.path.basename(f), package_md5(d), len(d))
    open(path, "w", encoding="utf-8", newline="").write(xml)
