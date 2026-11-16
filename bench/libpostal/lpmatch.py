"""Address matching on top of libpostal (parse + per-component dedupe). Policy: bench/POLICY.md."""
import unicodedata, re
from postal.parser import parse_address
from postal.expand import expand_address
from postal import dedupe as D

DS = D.duplicate_status
OK_STATUSES = {DS.EXACT_DUPLICATE, DS.LIKELY_DUPLICATE}
STRICT = {"road": OK_STATUSES, "house_number": OK_STATUSES, "unit": OK_STATUSES, "postcode": OK_STATUSES,
          "po_box": OK_STATUSES}

COUNTRY_LANGS = {"US": ["en"], "CA": ["en"], "GB": ["en"], "IE": ["en"], "AU": ["en"], "NZ": ["en"], "SG": ["en"], "ZA": ["en"],
                 "FR": ["fr"], "BE": ["fr", "nl"], "DE": ["de"], "AT": ["de"], "CH": ["de", "fr", "it"], "ES": ["es"], "MX": ["es"],
                 "AR": ["es"], "BR": ["pt"], "PT": ["pt"], "IT": ["it"], "NL": ["nl"], "PL": ["pl"], "CZ": ["cs"], "SE": ["sv"],
                 "NO": ["nb"], "DK": ["da"], "FI": ["fi"], "TR": ["tr"], "VN": ["vi"], "RU": ["ru"], "UA": ["uk"], "GR": ["el"],
                 "IL": ["he"], "SA": ["ar"], "AE": ["ar"], "EG": ["ar"], "IN": ["hi", "en"], "JP": ["ja"], "CN": ["zh"], "TW": ["zh"],
                 "KR": ["ko"], "TH": ["th"]}
DIGITS = {ord(c): str(i) for blk in ("٠١٢٣٤٥٦٧٨٩", "۰۱۲۳۴۵۶۷۸۹", "०१२३४५६७८९", "๐๑๒๓๔๕๖๗๘๙") for i, c in enumerate(blk)}
REVIEW_OK = __import__("os").environ.get("STREET_REVIEW", "0") == "1"
FUZZY = _os_env_fuzzy = __import__("os").environ.get("FUZZY", "1") == "1"

def prep(text):
    return unicodedata.normalize("NFKC", text).translate(DIGITS).replace("،", ",").replace("，", ",")

import json as _json, os as _os
ZIP = {}
_aid = _os.path.join(_os.path.dirname(__file__), "aid")
if _os.path.isdir(_aid):
    for f in _os.listdir(_aid):
        try:
            z = _json.load(open(_os.path.join(_aid, f))).get("zip")
            if z: ZIP[f[:2]] = re.compile("(?i)^(?:" + z + ")$")
        except Exception:
            pass

HN_PREFIX = re.compile(r"^(?:no|nº|n°|n|num|#|д|дом|буд|h\.?no|house|мак|मकान नं)[\s.:]*", re.I)
def hn_key(h):
    h = h.lower().strip()
    h = HN_PREFIX.sub("", h)
    h = re.sub(r"[\s.,:/-]+", "", h)
    h = h.replace("号", "").replace("號", "").replace("番地", "").replace("番", "-").replace("丁目", "-")
    return h

def pc_key(z):
    return re.sub(r"[\s-]+", "", z.lower().replace("〒", ""))

def pc_same(a, b):
    a, b = pc_key(a), pc_key(b)
    if a == b: return True
    if a.isdigit() and b.isdigit() and {len(a), len(b)} == {5, 9}: return a[:5] == b[:5]
    return False

def lev1(a, b):
    if a == b: return True
    if abs(len(a) - len(b)) > 1: return False
    if len(a) == len(b):
        d = [i for i in range(len(a)) if a[i] != b[i]]
        return len(d) == 1 or (len(d) == 2 and d[1] == d[0] + 1 and a[d[0]] == b[d[1]] and a[d[1]] == b[d[0]])
    if len(a) > len(b): a, b = b, a
    i = 0
    while i < len(a) and a[i] == b[i]: i += 1
    return a[i:] == b[i + 1:]

def fuzzy_street(a, b, langs):
    ea = expand_address(a, languages=langs) if langs else expand_address(a)
    eb = expand_address(b, languages=langs) if langs else expand_address(b)
    for x in ea:
        for y in eb:
            if len(x) >= 6 and lev1(x.replace(" ", ""), y.replace(" ", "")): return True
    return False

def parse(text, country=None):
    comps = {}
    kw = {"country": country.lower()} if country else {}
    for value, label in parse_address(prep(text), **kw):
        comps[label] = (comps[label] + " " + value) if label in comps else value
    # a postcode the parser glued onto the house number ("198 0021"): move it by the country's postcode pattern
    if country and "house_number" in comps and "postcode" not in comps and country in ZIP:
        toks = comps["house_number"].split()
        if len(toks) > 1 and ZIP[country].match(toks[-1]):
            comps["postcode"] = toks[-1]; comps["house_number"] = " ".join(toks[:-1])
    return comps

def _dup(fn, a, b, langs):
    try:
        return fn(a, b, languages=langs) if langs else fn(a, b)
    except Exception:
        return DS.NON_DUPLICATE

CITYISH = ("city", "suburb", "city_district", "town", "village", "island", "state_district")

def _topo_eq(a, b, langs):
    if a == b: return True
    try:
        st = D.is_toponym_duplicate(["city"], [a], ["city"], [b], languages=langs or None)
        if st in OK_STATUSES: return True
    except Exception:
        pass
    ea = set(expand_address(a, languages=langs) if langs else expand_address(a))
    eb = set(expand_address(b, languages=langs) if langs else expand_address(b))
    return bool(ea & eb)

def _toponym_same(pa, pb, langs):
    """None when either side names no locality; False when they contradict."""
    common = [k for k in CITYISH if k in pa and k in pb]
    if common:
        for k in common:
            if not _topo_eq(pa[k], pb[k], langs):
                # parser label drift: accept if a's value equals some other locality of b
                if not any(_topo_eq(pa[k], pb[j], langs) for j in CITYISH if j in pb and j != k):
                    return False
        return True
    la = [pa[k] for k in CITYISH if k in pa]; lb = [pb[k] for k in CITYISH if k in pb]
    if not la or not lb:
        return None
    return any(_topo_eq(x, y, langs) for x in la for y in lb)

def compare_parsed(pa, pb, langs, symmetric_unit=False):
    """pa: caller/query side, pb: candidate side. Returns (verdict, reasons)."""
    reasons = []
    if "po_box" in pa or "po_box" in pb:
        if "po_box" in pa and "po_box" in pb:
            if hn_key(re.sub(r"\D+", " ", pa["po_box"])) != hn_key(re.sub(r"\D+", " ", pb["po_box"])):
                return "different", ["po_box"]
            reasons.append("po_box")
        else:
            return "different", ["po_box_one_side"]
    else:
        if "road" not in pa or "road" not in pb:
            return "unknown", ["road_missing"]
        st = _dup(D.is_street_duplicate, pa["road"], pb["road"], langs)
        if st not in (OK_STATUSES | ({DS.NEEDS_REVIEW} if REVIEW_OK else set())) and not (FUZZY and fuzzy_street(pa["road"], pb["road"], langs)):
            return "different", ["road"]
        ha, hb = pa.get("house_number"), pb.get("house_number")
        if ha and hb:
            if hn_key(ha) != hn_key(hb):
                return "different", ["house_number"]
        elif ha or hb:
            return "different", ["house_number_one_side"]
    for lab in ("unit", "level", "staircase", "entrance"):
        ua, ub = pa.get(lab), pb.get(lab)
        if ua and ub:
            if re.sub(r"\D+", " ", ua).split() != re.sub(r"\D+", " ", ub).split():
                return "different", [lab]
        elif ub and not ua and not symmetric_unit:
            return "different", [lab + "_added"]
    za, zb = pa.get("postcode"), pb.get("postcode")
    if za and zb and not pc_same(za, zb):
        return "different", ["postcode"]
    t = _toponym_same(pa, pb, langs)
    if t is False:
        return "different", ["locality"]
    return "same", reasons

def compare(a, b, country=None):
    langs = COUNTRY_LANGS.get(country) if country else None
    return compare_parsed(parse(a, country), parse(b, country), langs)

def pick(query, candidates, country=None):
    langs = COUNTRY_LANGS.get(country) if country else None
    pq = parse(query, country)
    pcs = [parse(c, country) for c in candidates]
    verdicts = [compare_parsed(pq, pc, langs) for pc in pcs]
    same = [i for i, v in enumerate(verdicts) if v[0] == "same"]
    if not same:
        # same policy as the Rust matcher: the caller gave no unit and only unit listings fit -> ask
        added = [i for i, v in enumerate(verdicts) if v[0] == "different" and len(v[1]) == 1 and v[1][0].endswith("_added")]
        if added:
            return {"outcome": "ambiguous", "index": None, "same": added}
        return {"outcome": "none", "index": None, "same": []}
    # cluster same-place candidates
    parent = {i: i for i in same}
    def find(x):
        while parent[x] != x: x = parent[x]
        return x
    for x in same:
        for y in same:
            if x < y and find(x) != find(y):
                v1 = compare_parsed(pcs[x], pcs[y], langs, symmetric_unit=True)[0]
                if v1 == "same":
                    parent[find(y)] = find(x)
    clusters = {}
    for i in same: clusters.setdefault(find(i), []).append(i)
    if len(clusters) == 1:
        return {"outcome": "match", "index": same[0], "same": same}
    return {"outcome": "ambiguous", "index": None, "same": same}
