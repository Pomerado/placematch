#!/usr/bin/env python3
"""Build data/readings/han.txt from Unihan_Readings.txt (Unicode License v3; see LICENSE-UNICODE).

Usage: python3 -I tools/vendor-readings.py path/to/Unihan_Readings.txt > crates/placematch/data/readings/han.txt

One line per Han character in the Basic Multilingual Plane that has a reading:
  <char> TAB <Mandarin readings, toneless pinyin, space-separated> TAB <Japanese readings, lowercase Hepburn>
Japanese readings are kJapaneseOn and kJapaneseKun as listed; the matcher adds the stems and sound
changes that place names use.
"""
import sys, unicodedata, collections

def toneless(p):
    p = unicodedata.normalize("NFD", p)
    p = "".join(ch for ch in p if not unicodedata.combining(ch))
    return p.lower().replace("ü", "u")

man = collections.defaultdict(list)
jp = collections.defaultdict(list)
def add(d, ch, vals):
    for v in vals:
        if v and v not in d[ch]:
            d[ch].append(v)

for line in open(sys.argv[1], encoding="utf-8"):
    if line.startswith("#") or not line.strip():
        continue
    cp, field, val = line.rstrip("\n").split("\t", 2)
    ch = chr(int(cp[2:], 16))
    if ord(ch) > 0xFFFF:
        continue
    if field == "kMandarin":
        add(man, ch, [toneless(x) for x in val.split()])
    elif field == "kHanyuPinyin":
        for part in val.split():
            add(man, ch, [toneless(x) for x in part.split(":", 1)[1].split(",")])
    elif field in ("kJapaneseOn", "kJapaneseKun"):
        add(jp, ch, [x.lower() for x in val.split()])

for ch in sorted(set(man) | set(jp)):
    m = [x for x in man.get(ch, []) if x.isascii() and x.isalpha()]
    j = [x for x in jp.get(ch, []) if x.isascii() and x.isalpha()]
    if m or j:
        print(f"{ch}\t{' '.join(m)}\t{' '.join(j)}")
