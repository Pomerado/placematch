#!/usr/bin/env python3
"""Build match / no-match / ambiguous cases from REAL address records.

Usage: build_cases.py <real_dir> <out_dir> [--seed N]

Inputs (under real_dir): overture.jsonl (Overture Maps addresses, OpenAddresses-derived and government
sources, open licenses only), osm_*/addresses.jsonl + pois.jsonl + streets.jsonl (OpenStreetMap,
ODbL), divisions.jsonl (Overture divisions name translations, ODbL), hf_ko/gil.json (Korean road names
with official romanization, MIT).

Every case starts from one real address. The caller text and the site's listings are renderings of
real records with perturbations a caller or a site would make. Distractors are real neighbours: the
same street with another number, another street in the same town, the same street name in another
town, another postcode in the same town, other businesses. The split is by source region: a whole
region (state, province, department, dataset or city) is either tune or held-out, and some countries
are held-out entirely.
"""
import json, os, re, sys, random, hashlib, collections, unicodedata
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from conventions import conv, ABBR, SUFFIX_GLUE, SUFFIX_GLUE_ABBR, UNIT_FAMILIES, COMBINING

REAL = sys.argv[1]; OUT = sys.argv[2]
SEED = int(sys.argv[sys.argv.index("--seed") + 1]) if "--seed" in sys.argv else 11
# OpenStreetMap samples: every <real_dir>/osm_* directory written by fetch_osm.sh (several runs may share the work)
OSM_DIRS = sorted(os.path.join(REAL, d) for d in os.listdir(REAL) if d.startswith("osm_") and os.path.isdir(os.path.join(REAL, d)))
CAP_ADDR = 1700; CAP_POI = 700
HELDOUT_COUNTRIES = {"NZ", "CO", "UY", "SK", "SI", "EE", "LV", "IS", "LU", "NC", "HK",
                     "JO", "QA", "KW", "LB", "BY", "KZ", "CY", "GE", "AM", "PH", "MY", "HU", "RO", "ID"}

# ---------------------------------------------------------------- text helpers
def script_of(s):
    cnt = collections.Counter()
    for ch in s or "":
        o = ord(ch)
        if ch.isdigit() or not ch.isalpha(): continue
        if o < 0x250 or 0x1E00 <= o < 0x1F00: cnt["Latn"] += 1
        elif 0x370 <= o < 0x400 or 0x1F00 <= o < 0x2000: cnt["Grek"] += 1
        elif 0x400 <= o < 0x530: cnt["Cyrl"] += 1
        elif 0x530 <= o < 0x590: cnt["Armn"] += 1
        elif 0x590 <= o < 0x600: cnt["Hebr"] += 1
        elif 0x600 <= o < 0x700 or 0x750 <= o < 0x780 or 0xFB50 <= o < 0xFF00: cnt["Arab"] += 1
        elif 0x900 <= o < 0x980: cnt["Deva"] += 1
        elif 0xE00 <= o < 0xE80: cnt["Thai"] += 1
        elif 0x10A0 <= o < 0x1100: cnt["Geor"] += 1
        elif 0x3040 <= o < 0x3100: cnt["Kana"] += 1
        elif 0xAC00 <= o < 0xD7B0 or 0x1100 <= o < 0x1200 or 0x3130 <= o < 0x3190: cnt["Hang"] += 1
        elif 0x4E00 <= o < 0xA000 or 0x3400 <= o < 0x4DC0: cnt["Hani"] += 1
        else: cnt["Other"] += 1
    if not cnt: return "Latn"
    return cnt.most_common(1)[0][0]

def deaccent(s):
    s = s.replace("ß", "ss").replace("ø", "o").replace("Ø", "O").replace("ł", "l").replace("Ł", "L").replace("đ", "d").replace("Đ", "D")
    return unicodedata.normalize("NFC", COMBINING.sub("", unicodedata.normalize("NFD", s)))

SMALL = {"de", "da", "do", "dos", "das", "del", "la", "le", "les", "du", "des", "di", "y", "e", "van", "der", "den", "von", "und", "et", "el"}
def smart_title(s):
    out = []
    for i, w in enumerate(s.split(" ")):
        lw = w.lower()
        if i > 0 and lw in SMALL: out.append(lw); continue
        out.append("-".join(p[:1].upper() + p[1:].lower() if p else p for p in lw.split("-")))
    return " ".join(out)

def is_upper_src(s):
    letters = [c for c in s if c.isalpha() and c.isascii()]
    return len(letters) >= 3 and all(c.isupper() for c in letters)

def case_like(abbr, word):
    if word.isupper() and len(word) > 1: return abbr.upper()
    if word[:1].islower(): return abbr[:1].lower() + abbr[1:]
    return abbr

NUMRE = re.compile(r"^(\d+)\s*([A-Za-z]|bis|ter)?$")
def simple_number(n):
    return bool(NUMRE.match(n or ""))

def numkey(n):
    return re.sub(r"[\s號号番地]+", "", unicodedata.normalize("NFKC", n or "").lower())

def clean_pc(p):
    """A postcode, or None when the source masked or garbled it ("42***")."""
    p = (p or "").strip()
    return p if p and re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9 -]{1,11}", p) and any(ch.isdigit() for ch in p) else None

def key(s):
    # 臺 and 台 are one character in Taiwanese place names
    return re.sub(r"[\W_]+", "", deaccent((s or "").lower().replace("臺", "台")))

# ---------------------------------------------------------------- loaders
def load_divisions():
    tr = collections.defaultdict(dict)
    p = os.path.join(REAL, "divisions.jsonl")
    if not os.path.exists(p): return tr
    for l in open(p):
        d = json.loads(l)
        nm = d["names"]; prim = d["primary"]
        en = nm.get("ja-Latn") if d["country"] == "JP" and nm.get("ja-Latn") else (nm.get("ko-Latn") if d["country"] == "KR" and nm.get("ko-Latn") else nm.get("en"))
        if not en or not prim: continue
        for name in {prim, nm.get("ja"), nm.get("ko"), nm.get("ru"), nm.get("uk"), nm.get("ar"), nm.get("he"), nm.get("el"), nm.get("th"), nm.get("zh-Hant"), nm.get("bg"), nm.get("sr")}:
            if name and name != en and (d["country"], name) not in tr:
                tr[(d["country"], name)] = en
    return tr

def load_streets_alt():
    alt = {}
    for p in [os.path.join(d, "streets.jsonl") for d in OSM_DIRS]:
        if not os.path.exists(p): continue
        for l in open(p):
            d = json.loads(l)
            en = (d.get("names") or {}).get("en")
            if en and d.get("name") and en != d["name"]:
                alt.setdefault((d["country"], d["name"]), en)
    p = os.path.join(REAL, "hf_ko", "gil.json")
    if os.path.exists(p):
        for d in json.load(open(p)):
            alt.setdefault(("KR", d["KO"]), d["EN"])
    return alt

UNITRE = re.compile(r"^[0-9A-Za-z]{1,5}$")
def load_records():
    recs = []
    p = os.path.join(REAL, "overture.jsonl")
    for l in open(p):
        d = json.loads(l)
        lv = d.get("levels") or []
        city = d.get("postal_city") or (lv[-1] if lv else None)
        region = lv[0] if len(lv) >= 2 else None
        district = lv[-2] if len(lv) >= 3 else None
        if d["country"] == "TW" and len(lv) >= 2:
            city, district, region = lv[0], lv[1], None
        if d["country"] == "NZ" and len(lv) >= 2:
            city, district, region = lv[0], lv[1], None
        if d["country"] in ("US", "CA", "AU", "BR", "MX", "DE", "BE", "CO", "CL", "JP") and len(lv) == 1 and not d.get("postal_city"):
            city, region = None, lv[0]
        if d["country"] == "LV": continue
        ds = d.get("dataset") or ""
        rkey = ds if ds and not ds.endswith("countrywide") else (lv[0] if lv else ds)
        unit = d.get("unit") if d.get("unit") and UNITRE.match(d["unit"]) else None
        recs.append({"src": "overture", "license": d["license"], "dataset": ds, "country": d["country"], "number": d["number"].strip(),
                     "street": d["street"].strip(), "unit": unit, "postcode": clean_pc(d.get("postcode")), "city": city, "region": region,
                     "district": district, "name": None, "rkey": f"{d['country']}:{rkey}"})
    seen_ids = set()
    for sub, fn, kind in [(sub, fn, kind) for sub in OSM_DIRS for fn, kind in (("addresses.jsonl", "address"), ("pois.jsonl", "poi"))]:
        p = os.path.join(sub, fn)
        if not os.path.exists(p): continue
        for l in open(p):
            d = json.loads(l)
            if (kind, d.get("osm_id")) in seen_ids: continue
            seen_ids.add((kind, d.get("osm_id")))
            street = d.get("street") or d.get("place")
            if not street or not d.get("number"): continue
            if kind == "poi" and not d.get("name"): continue
            unit = d.get("unit") if d.get("unit") and UNITRE.match(str(d["unit"])) else None
            if ";" in str(d["number"]) or ";" in street: continue
            pc = (d.get("postcode") or "").strip()
            area = d.get("province") or (pc[:2] if pc[:2].isdigit() else None) or d.get("city") or d.get("city_query")
            recs.append({"src": "osm", "license": "ODbL-1.0", "dataset": "osm", "country": d["country"], "number": str(d["number"]).strip(),
                         "street": street.strip(), "unit": unit, "postcode": clean_pc(d.get("postcode")), "city": d.get("city") or None,
                         "city_query": d.get("city_query"), "region": d.get("province"), "district": d.get("district"),
                         "name": d.get("name") if kind == "poi" else None, "rkey": f"{d['country']}:{area}", "kind": kind})
    return recs

# ---------------------------------------------------------------- rendering
def lang_of(rec):
    langs = conv(rec["country"])[3]
    st = rec["street"].lower()
    for lg in langs:
        for full, _ in ABBR.get(lg, []):
            if re.search(r"(^|\s)" + re.escape(full) + r"(\s|$)", st): return lg
        for suf in SUFFIX_GLUE.get(lg, []):
            if re.search(re.escape(suf) + r"(\s|$)", st): return lg
    return langs[0]

def abbreviate(street, lang, rng, site=False):
    words = street.split(" ")
    table = {f: a for f, a in ABBR.get(lang, [])}
    cands = [i for i, w in enumerate(words) if w.lower() in table]
    out = list(words)
    changed = False
    for i in cands:
        if rng.random() < (0.8 if not site else 0.6):
            a = rng.choice(table[words[i].lower()])
            out[i] = case_like(a, words[i]); changed = True
    if not changed:
        for suf, abbrs in SUFFIX_GLUE_ABBR.items():
            for i, w in enumerate(out):
                lw = w.lower()
                if lw.endswith(suf) and len(lw) > len(suf) + 2 and suf in SUFFIX_GLUE.get(lang, []):
                    a = rng.choice(abbrs)
                    out[i] = w[: len(w) - len(suf)] + case_like(a, w[len(w) - len(suf):]); changed = True; break
            if changed: break
    return " ".join(out), changed

def typo(s, rng):
    words = s.split(" ")
    idx = [i for i, w in enumerate(words) if sum(c.isalpha() for c in w) >= 6 and w.lower() not in TYPE_WORDS]
    if not idx: return s, False
    i = rng.choice(idx); w = list(words[i]); k = rng.randrange(1, len(w) - 1)
    op = rng.choice(["sub", "del", "ins", "swap"])
    if not w[k].isalpha(): return s, False
    letters = "aeiounrstl" if w[k].isascii() else w[k]
    if op == "sub": w[k] = rng.choice([c for c in letters if c != w[k].lower()] or ["x"])
    elif op == "del": del w[k]
    elif op == "ins": w.insert(k, rng.choice(letters))
    else:
        if k + 1 < len(w) and w[k] != w[k + 1] and w[k + 1].isalpha(): w[k], w[k + 1] = w[k + 1], w[k]
        else: return s, False
    words[i] = "".join(w)
    return " ".join(words), True

NATIVE_DIGITS = {"Arab": "٠١٢٣٤٥٦٧٨٩", "Deva": "०१२३४५६७८९", "Thai": "๐๑๒๓๔๕๖๗๘๙", "Jpan": "０１２３４５６７８９", "Hani": "０１２３４５６７８９"}
def to_native_digits(s, scr):
    t = NATIVE_DIGITS[scr]
    return "".join(t[int(c)] if c.isascii() and c.isdigit() else c for c in s)

KANJI_NUM = {"一": 1, "二": 2, "三": 3, "四": 4, "五": 5, "六": 6, "七": 7, "八": 8, "九": 9, "十": 10}
def kanji_to_int(s):
    if not s: return None
    if s == "十": return 10
    if len(s) == 2 and s[0] == "十": return 10 + KANJI_NUM.get(s[1], 0)
    if len(s) == 2 and s[1] == "十": return KANJI_NUM.get(s[0], 0) * 10
    if len(s) == 3 and s[1] == "十": return KANJI_NUM.get(s[0], 0) * 10 + KANJI_NUM.get(s[2], 0)
    if len(s) == 1: return KANJI_NUM.get(s)
    return None

JP_CHOME = re.compile(r"^(.*?)([一二三四五六七八九十]+)丁目$")

class Ctx:
    def __init__(self, divs, salt):
        self.divs = divs; self.salt = salt

def render(rec, ctx, rng, opts, tags):
    """opts: dict of rendering choices; tags: set to fill with applied, visible perturbations."""
    cc = rec["country"]
    npos, ppos, rstyle, langs, cnames = conv(cc)
    street = rec["street"]; number = rec["number"]; city = rec.get("city"); postcode = rec.get("postcode")
    if cc == "CO" and " " in number:
        number = "# " + re.sub(r"\s+(\d+[A-Za-z]?)$", r"-\1", number)
    region = rec.get("region"); unit = rec.get("unit"); name = rec.get("name")
    lang = lang_of(rec)
    scr = script_of(street)
    # site style: title-case all-caps sources most of the time
    if opts.get("title_src") and is_upper_src(street):
        street = smart_title(street)
        city = smart_title(city) if city and is_upper_src(city) else city
        region = smart_title(region) if region and is_upper_src(region) and len(region) > 3 else region
    if opts.get("translit"):
        alt = opts["translit"]
        street = alt.get("street") or street
        if alt.get("city"): city = alt["city"]
        region = alt.get("region")
        opts = dict(opts, district=False)
        tags.add("translit"); scr_out = "Latn"
    if opts.get("exonym") and city and npos not in ("big", "after_big"):
        street_city = ctx.divs.get((cc, city))
        if street_city and (re.search(r"(?i)municipal|district|county|prefecture|province|city of|region|parish|commune|arrondissement|ward|borough|[(]|[0-9]", street_city) or re.search(r"[0-9]", city) or len(street_city) > 2 * len(city) + 4):
            street_city = None
        if street_city and key(street_city) != key(city):
            city = street_city; tags.add("exonym")
    if opts.get("typo") and scr not in ("Hani", "Kana", "Hang", "Thai"):
        street2, ch = typo(street, rng)
        if ch: street = street2; tags.add("typo")
    if opts.get("abbrev") and not opts.get("translit"):
        street, ch = abbreviate(street, lang, rng, site=opts.get("site", False))
        if ch: tags.add("abbrev_site" if opts.get("site") else "abbrev")
    if not opts.get("postcode", True) and postcode: postcode = None; tags.add("postcode_dropped")
    if not opts.get("city", True) and city: city = None; tags.add("city_dropped")
    if not opts.get("region", True): region = None
    if not opts.get("unit", True) and unit: unit = None; tags.add("unit_dropped_c" if opts.get("site") else "unit_dropped_q")
    if opts.get("add_unit"): unit = opts["add_unit"]
    big = npos in ("big", "after_big") and not opts.get("translit")
    parts = []
    if big:
        text = render_big(rec, street, number, unit, city, region, postcode, opts, rng, tags)
        parts = [text]
    else:
        pos = npos if npos in ("before", "after") else "before"
        if opts.get("reorder") and pos in ("before", "after") and scr in ("Latn", "Cyrl", "Grek") and cc not in ("CO",) and not re.search(r"\d", street):
            pos = "after" if pos == "before" else "before"; tags.add("reorder")
        if opts.get("translit") and cc in ("JP", "KR", "CN", "TW"): pos = "before"
        line = f"{number} {street}" if pos == "before" else f"{street} {number}"
        if cc in ("TW", "CN") and opts.get("translit"): line = f"No. {number}, {street}"
        parts.append(line)
        if unit:
            fams = UNIT_FAMILIES.get(lang, UNIT_FAMILIES["en"]) if not opts.get("translit") else UNIT_FAMILIES["en"]
            fam = fams[h(rec["street"] + rec["number"]) % len(fams)]
            uw = rng.choice(fam)
            if cc == "AU" and rng.random() < 0.4 and pos == "before": parts[0] = f"{unit}/{line}"
            else: parts.append(uw.format(u=unit))
        d = rec.get("district")
        if d and opts.get("district") and city and key(d)[:5] != key(city)[:5]: parts.append(d)
        loc = []
        if ppos == "after_region":
            seg = city or ""
            if region and opts.get("region", True): seg = f"{seg}, {region}" if seg else region
            if postcode: seg = f"{seg} {postcode}".strip()
            if seg: loc.append(seg)
        else:
            pc_first = ppos == "before"
            if opts.get("pc_swap") and postcode and city: pc_first = not pc_first; tags.add("reorder_pc")
            if city and postcode: loc.append(f"{postcode} {city}" if pc_first else f"{city} {postcode}")
            elif city: loc.append(city)
            elif postcode: loc.append(postcode)
            if region and opts.get("region", True) and region != city: loc.append(region)
        parts += loc
        if opts.get("country"): parts.append(cnames[0] if not opts.get("translit") else cnames[-1])
        if opts.get("big_order"):  # Russian / Eastern European large-to-small order
            parts = parts[1:][::-1] + parts[:1]; tags.add("reorder")
    if name and opts.get("business"):
        parts.insert(0, name); tags.add("business_query" if not opts.get("site") else "business")
    sep = ", "
    if opts.get("no_commas"): sep = " "; tags.add("no_commas")
    text = sep.join(p for p in parts if p)
    if opts.get("case") == "lower": text = text.lower(); tags.add("case")
    elif opts.get("case") == "upper": text = text.upper(); tags.add("case")
    if opts.get("deaccent"):
        t2 = deaccent(text)
        if t2 != text: text = t2; tags.add("diacritics")
    if opts.get("native_digits"):
        sc = "Jpan" if cc == "JP" else script_of(rec["street"])
        if sc in NATIVE_DIGITS:
            t2 = to_native_digits(text, sc)
            if t2 != text: text = t2; tags.add("digits_native")
    return text

def render_big(rec, street, number, unit, city, region, postcode, opts, rng, tags):
    cc = rec["country"]
    if cc == "JP":
        m = JP_CHOME.match(street)
        style = opts.get("jp_style") or "kanji"
        if m and style in ("arabic", "dash"):
            n = kanji_to_int(m.group(2))
            if n:
                if style == "arabic": street = f"{m.group(1)}{n}丁目"; num = f"{number}"
                else: street = f"{m.group(1)}{n}"; num = f"-{number}"
                tags.add("jp_form")
            else: num = number
        else: num = number + ("番地" if opts.get("jp_banchi") else "")
        if unit: num += f" {unit}号室"
        pc = f"〒{postcode} " if postcode else ""
        return f"{pc}{region or ''}{city or ''}{street}{num}"
    if cc in ("CN", "TW"):
        num = number if number.endswith(("号", "號")) else number + ("號" if cc == "TW" else "号")
        u = f"{unit}室" if unit else ""
        return f"{postcode + ' ' if postcode and opts.get('postcode', True) else ''}{region or ''}{city or ''}{rec.get('district') or '' if opts.get('district') else ''}{street}{num}{u}"
    if cc == "KR":
        u = f" {unit}호" if unit else ""
        head = " ".join(x for x in (region, city) if x)
        return f"{head + ' ' if head else ''}{street} {number}{u}" + (f" ({postcode})" if postcode else "")
    return f"{street} {number}"

# ---------------------------------------------------------------- case building
def h(s):
    return int(hashlib.sha1(s.encode()).hexdigest()[:8], 16)

def caller_opts(rec, rng, alt):
    o = {"title_src": rng.random() < 0.85, "postcode": rng.random() < 0.45, "region": rng.random() < 0.4,
         "country": rng.random() < 0.15, "abbrev": rng.random() < 0.45, "deaccent": rng.random() < 0.25,
         "no_commas": rng.random() < 0.12, "reorder": rng.random() < 0.15, "typo": rng.random() < 0.08,
         "pc_swap": rng.random() < 0.08, "district": rng.random() < 0.15, "native_digits": rng.random() < 0.15,
         "exonym": rng.random() < 0.3, "big_order": rec["country"] in ("RU", "UA", "BY", "KZ", "BG") and rng.random() < 0.15,
         "jp_style": rng.choice(["kanji", "arabic", "dash"]), "jp_banchi": rng.random() < 0.3}
    r = rng.random()
    o["case"] = "lower" if r < 0.12 else ("upper" if r < 0.17 else None)
    if alt and rng.random() < 0.35: o["translit"] = alt
    return o

def site_opts(rec, rng):
    return {"site": True, "title_src": rng.random() < 0.7, "postcode": True, "region": rng.random() < 0.5,
            "country": rng.random() < 0.3, "abbrev": rng.random() < 0.2, "district": rng.random() < 0.2,
            "jp_style": rng.choice(["kanji", "kanji", "arabic", "dash"]), "jp_banchi": rng.random() < 0.3}

class Pool:
    def __init__(self, recs):
        self.by_cs = collections.defaultdict(list)      # (cc, citykey, streetkey) -> recs
        self.by_city = collections.defaultdict(list)    # (cc, citykey) -> recs
        self.street_cities = collections.defaultdict(dict)  # (cc, streetkey) -> {citykey: rec}
        self.core = collections.defaultdict(list)       # (cc, citykey, core name) -> recs  (street type variants)
        towns = collections.defaultdict(dict)
        for r in recs:
            ck = key(r.get("city") or r.get("postcode") or r.get("city_query") or "")
            sk = key(r["street"])
            r["_ck"] = ck; r["_sk"] = sk
            self.by_cs[(r["country"], ck, sk)].append(r)
            self.by_city[(r["country"], ck)].append(r)
            if r.get("city"): self.street_cities[(r["country"], sk)].setdefault(ck, r)
            core = core_name(r["street"], r["country"])
            if core: self.core[(r["country"], ck, key(core))].append(r)
            if r.get("city"): towns[r["rkey"]].setdefault(ck, r)
        self.region_towns = {k: list(v.values()) for k, v in towns.items()}

TYPE_WORDS = set()
for lg, rows in ABBR.items():
    for f, _ in rows:
        if f not in ("saint", "san", "santa", "são", "general", "doctor", "doutor", "professor", "north", "south", "east", "west"): TYPE_WORDS.add(f)
def core_name(street, cc):
    ws = [w for w in street.lower().split(" ") if w]
    rest = [w for w in ws if w not in TYPE_WORDS]
    if len(rest) == len(ws) or not rest: return None
    return " ".join(rest)

def other(rec, **kw):
    r = dict(rec); r.update(kw); return r

DIVS = {}   # (country, native name) -> name in English or Latin script, set in main()
try:
    from unidecode import unidecode
except ImportError:  # transliteration check is then skipped
    unidecode = None

def same_town(a, b, cc):
    """Two spellings of one town: equal keys, one inside the other, one transliteration, or a translation."""
    ka, kb = key(a), key(b)
    if not ka or not kb or ka == kb or ka in kb or kb in ka: return True
    if unidecode and key(unidecode(a)) == key(unidecode(b)): return True
    ta, tb = DIVS.get((cc, a)), DIVS.get((cc, b))
    return any(x and y and key(x) == key(y) for x, y in ((ta, b), (a, tb), (ta, tb)))

# a ward, district or subdistrict name: it may lie inside the other town, so it is no proof of another place
SUBTOWN = re.compile(r"(?i)^(phường|phuong|quận|quan|xã|thị trấn|huyện|ward|district|subdistrict|ตำบล|อำเภอ|แขวง|เขต|ต\.|อ\.)\b|\b(ward|district|subdistrict|mahallesi|köyü)$|^\d+$")

def far(rec, r):
    """r is in another town than rec (not just another spelling of the same town, nor a district of it)."""
    ck = rec["_ck"]; c2 = key(r.get("city") or "")
    if not c2 or not ck: return False
    if SUBTOWN.search((rec.get("city") or "").strip()) or SUBTOWN.search((r.get("city") or "").strip()): return False
    if same_town(rec.get("city") or "", r.get("city") or "", rec["country"]): return False
    for d in (rec.get("district"), r.get("district")):
        if d and (key(d) in (ck, c2)): return False
    if r.get("postcode") and rec.get("postcode") and key(r["postcode"]) == key(rec["postcode"]): return False
    return True

def near_name(a, b):
    """Within one edit: a caller's one-letter typo would make these the same street under the policy."""
    a, b = key(a), key(b)
    if abs(len(a) - len(b)) > 1: return False
    if a == b: return True
    i = 0
    while i < min(len(a), len(b)) and a[i] == b[i]: i += 1
    return a[i + 1:] == b[i + 1:] or a[i:] == b[i + 1:] or a[i + 1:] == b[i:] or (a[i:i + 2] == b[i:i + 2][::-1] and a[i + 2:] == b[i + 2:])

STREET_TYPES = {"en": ["street", "avenue", "road", "drive", "lane", "court", "place", "boulevard", "way", "circle", "terrace"],
                "fr": ["rue", "avenue", "boulevard", "chemin", "impasse", "allée", "place", "route"],
                "es": ["calle", "avenida", "paseo", "camino", "plaza", "carrera", "pasaje"],
                "pt": ["rua", "avenida", "travessa", "praça", "estrada", "alameda", "largo"],
                "it": ["via", "viale", "piazza", "corso", "vicolo", "largo", "strada"],
                "ru": ["улица", "проспект", "переулок", "бульвар", "проезд"], "uk": ["вулиця", "проспект", "провулок", "бульвар"],
                "pl": ["ulica", "aleja", "plac", "osiedle"], "tr": ["caddesi", "sokak", "bulvarı"],
                "el": ["οδός", "λεωφόρος", "πλατεία"], "ar": ["شارع", "طريق"], "th": ["ถนน", "ซอย"], "ro": ["strada", "bulevardul", "calea", "aleea"],
                "cs": ["ulice", "náměstí", "třída"], "hu": ["utca", "út", "tér", "körút"], "lt": ["gatvė", "prospektas", "alėja"]}
def swap_type(street, lang, rng):
    words = street.split(" ")
    types = STREET_TYPES.get(lang, [])
    for i, w in enumerate(words):
        if w.lower() in types:
            alt = rng.choice([t for t in types if t != w.lower()])
            words[i] = alt.upper() if w.isupper() else (alt.capitalize() if w[:1].isupper() else alt)
            return " ".join(words)
    glue = SUFFIX_GLUE.get(lang, [])
    same = [{"gate", "gata"}, {"veien", "vegen", "vei"}, {"straße", "strasse"}, {"vej", "vei"}]
    for i, w in enumerate(words):
        lw = w.lower()
        for suf in sorted(glue, key=len, reverse=True):
            if lw.endswith(suf) and len(lw) > len(suf) + 2:
                eq = set().union(*[g for g in same if suf in g]) | {suf}
                alts = [t for t in glue if t not in eq and not any(t in g and suf in g for g in same)]
                if not alts: return None
                alt = rng.choice(alts)
                words[i] = w[: len(w) - len(suf)] + (alt.upper() if w.isupper() else alt)
                return " ".join(words)
    return None

def distractors(rec, pool, rng):
    """Real near neighbours of rec, each with the tag of the hard component it changes."""
    out = []
    cc, ck, sk = rec["country"], rec["_ck"], rec["_sk"]
    same_street = [r for r in pool.by_cs[(cc, ck, sk)] if numkey(r["number"]) != numkey(rec["number"])]
    m = NUMRE.match(rec["number"])
    for r in same_street:
        m2 = NUMRE.match(r["number"])
        if m and m2 and m.group(1) == m2.group(1): out.append(("neg_suffix", other(rec, number=r["number"], unit=None, name=None)))
        else: out.append(("neg_number", other(rec, number=r["number"], unit=None, name=None)))
    city_recs = pool.by_city[(cc, ck)]
    if len(city_recs) > 1:
        mycore = key(core_name(rec["street"], cc) or rec["street"])
        def distinct(r):
            oc2 = key(core_name(r["street"], cc) or r["street"])
            bare = lambda x: key(" ".join(w for w in x.lower().split() if w not in SMALL))
            return (r["_sk"] != sk and oc2 != mycore and oc2 not in mycore and mycore not in oc2 and not near_name(r["street"], rec["street"])
                    and bare(r["street"]) != bare(rec["street"]))
        same_num = [r for r in city_recs if numkey(r["number"]) == numkey(rec["number"]) and distinct(r)]
        for r in same_num[:3]: out.append(("neg_street_name", other(rec, street=r["street"], name=None, unit=None, postcode=r.get("postcode") or rec.get("postcode"))))
        for _ in range(2):
            r = rng.choice(city_recs)
            if distinct(r): out.append(("neg_street_name", other(rec, street=r["street"], name=None, unit=None, postcode=r.get("postcode") or rec.get("postcode"))))
        # another postcode, not the same code written with or without its letters ("S2322" / "2322")
        digits = lambda p: re.sub(r"\D", "", p or "")
        pcs = {r.get("postcode") for r in city_recs if r.get("postcode") and key(r["postcode"]) != key(rec.get("postcode") or "")
               and digits(r["postcode"]) != digits(rec.get("postcode")) and not digits(rec.get("postcode") or "x").startswith(digits(r["postcode"]) or "y")}
        if rec.get("postcode") and pcs: out.append(("neg_postcode", other(rec, postcode=rng.choice(sorted(pcs)), name=None)))
    core = core_name(rec["street"], cc)
    found = False
    if core:
        for r in pool.core[(cc, ck, key(core))]:
            if r["_sk"] != sk: out.append(("neg_street_type", other(rec, street=r["street"], name=None, unit=None))); found = True; break
    if not found and rng.random() < 0.5:
        st2 = swap_type(rec["street"], lang_of(rec), rng)
        if st2 and key(st2) != sk: out.append(("neg_street_type", other(rec, street=st2, name=None, unit=None)))
    cities = pool.street_cities.get((cc, sk), {})
    oc = [r for c, r in cities.items() if far(rec, r)]
    if not oc:
        near = pool.region_towns.get(rec["rkey"], [])
        if len(near) > 1:
            r = rng.choice(near)
            if far(rec, r): oc = [r]
    for r in oc[:2]: out.append(("neg_city", other(rec, city=r["city"], postcode=r.get("postcode"), region=r.get("region"), district=None, name=None)))
    if rec.get("unit"):
        u = str(rec["unit"])
        m3 = re.match(r"^(.*?)(\d+)$", u)
        if m3: u2 = m3.group(1) + str(int(m3.group(2)) + rng.randint(1, 9))
        elif len(u) == 1 and u.isalpha(): u2 = chr(ord(u.upper()) + 1) if u.upper() < "Z" else "A"
        else: u2 = None
        if u2 and u2.lower() != u.lower(): out.append(("neg_unit", other(rec, unit=u2)))
    rng.shuffle(out)
    first, seen = [], set()
    for x in out:
        if x[0] not in seen: first.append(x); seen.add(x[0])
    return first + [x for x in out if x not in first]

def make_case(rec, pool, ctx, rng, alts, cid):
    cc = rec["country"]
    alt = alts(rec)
    poi = bool(rec.get("name"))
    kind = rng.random()
    dis = distractors(rec, pool, rng)
    cities = pool.street_cities.get((cc, rec["_sk"]), {})
    other_city = [r for c, r in cities.items() if far(rec, r)]
    if not other_city and rng.random() < 0.5:
        near = [r for r in pool.region_towns.get(rec["rkey"], []) if far(rec, r)]
        if near: other_city = [rng.choice(near)]
    tags = set()
    qo = caller_opts(rec, rng, alt)
    if poi: qo["business"] = rng.random() < 0.35
    base = {"id": cid, "country": cc, "script": script_of(rec["street"]), "lang": lang_of(rec), "src": rec["src"],
            "license": rec["license"], "region_key": rec["rkey"]}
    if base["script"] in ("Hani", "Kana") and cc == "JP": base["script"] = "Jpan"
    def S(r, extra=None, true=False):
        o = site_opts(r, rng)
        if poi and r is rec: o["business"] = rng.random() < 0.6
        if extra: o.update(extra)
        return render(r, ctx, rng, o, tags if true else set())
    def shuffled(items):
        """items: list of (text, role). Returns texts, roles."""
        rng.shuffle(items)
        return [x for x, _ in items], [r for _, r in items]
    # ambiguous: caller gives no town, the street+number exists in two towns
    if kind < 0.12 and other_city:
        # no business name either: a listing carrying it would tell the two places apart
        qo.update(city=False, postcode=False, region=False, district=False, exonym=False, country=False, business=False)
        q = render(rec, ctx, rng, qo, tags)
        o = rng.choice(other_city)
        twin = other(rec, city=o["city"], postcode=o.get("postcode"), region=o.get("region"), district=None, name=None, unit=rec.get("unit"))
        items = [(S(rec, true=True), "true"), (S(twin, {"business": False}), "twin_city")]
        for t, d in dis[: rng.randint(0, 2)]:
            if t in ("neg_number", "neg_street_name", "neg_street_type", "neg_suffix"): items.append((S(d, {"business": False}), t))
        cands, roles = shuffled(items)
        same = [i for i, r in enumerate(roles) if r in ("true", "twin_city")]
        tags.add("amb_city")
        return dict(base, task="pick", query=q, candidates=cands, roles=roles, expected="ambiguous", accept=[], same=same, tags=sorted(tags))
    q = render(rec, ctx, rng, qo, tags)
    # a distractor is only a different place if the caller gave the component it changes
    if not (qo.get("postcode") and rec.get("postcode")): dis = [x for x in dis if x[0] != "neg_postcode"]
    if not rec.get("city") or qo.get("city") is False: dis = [x for x in dis if x[0] != "neg_city"]
    if kind < 0.40:  # pair positive
        o = {}
        if rec.get("unit") and rng.random() < 0.3: o["unit"] = False
        c = S(rec, o, true=True)
        return dict(base, task="pair", query=q, candidates=[c], roles=["true"], expected="match", accept=[0], same=[], tags=sorted(tags | {"pos"}))
    if kind < 0.62 and dis:  # pair negative
        types = sorted({x[0] for x in dis})
        tt = rng.choice(types)
        t, d = next(x for x in dis if x[0] == tt)
        if not rec.get("unit") and rng.random() < 0.12:
            t, d = "neg_unit_added", other(rec, unit=str(rng.randint(1, 40)))
        c = S(d, {"business": False})
        return dict(base, task="pair", query=q, candidates=[c], roles=[t], expected="none", accept=[], same=[], tags=sorted(tags | {t}))
    if kind < 0.70 and not rec.get("unit") and not qo.get("translit"):  # caller gave no unit; the site only lists units
        items = [(S(other(rec, unit=str(u)), {"business": False}), "unit_added") for u in rng.sample(range(1, 60), rng.randint(1, 3))]
        for t, d in dis[: rng.randint(0, 2)]:
            if t in ("neg_number", "neg_street_name", "neg_city"): items.append((S(d, {"business": False}), t))
        cands, roles = shuffled(items)
        tags.add("unit_added_only")
        return dict(base, task="pick", query=q, candidates=cands, roles=roles, expected="ambiguous", accept=[], same=[i for i, r in enumerate(roles) if r == "unit_added"], tags=sorted(tags))
    if kind < 0.90 and dis:  # pick with the right one listed
        k = rng.randint(1, min(5, len(dis)))
        items = [(S(d, {"business": rng.random() < 0.2 and poi}), t) for t, d in dis[:k]]
        items.append((S(rec, true=True), "true"))
        if rng.random() < 0.15: items.append((S(rec, {"business": poi, "abbrev": True}), "true_variant"))
        cands, roles = shuffled(items)
        for t, _ in dis[:k]: tags.add("d_" + t)
        accept = [i for i, r in enumerate(roles) if r.startswith("true")]
        return dict(base, task="pick", query=q, candidates=cands, roles=roles, expected="match", accept=accept, same=[], tags=sorted(tags | {"pick_match"}))
    if dis:  # pick with only distractors
        k = rng.randint(1, min(5, len(dis)))
        cands, roles = shuffled([(S(d, {"business": False}), t) for t, d in dis[:k]])
        for t in roles: tags.add("d_" + t)
        return dict(base, task="pick", query=q, candidates=cands, roles=roles, expected="none", accept=[], same=[], tags=sorted(tags | {"pick_none"}))
    c = S(rec, true=True)
    return dict(base, task="pair", query=q, candidates=[c], roles=["true"], expected="match", accept=[0], same=[], tags=sorted(tags | {"pos"}))

def main():
    rng = random.Random(SEED)
    divs = load_divisions(); salt = SEED
    DIVS.update(divs)
    street_alt = load_streets_alt()
    recs = load_records()
    # drop records with no digit in the number, very long numbers, or no locality at all
    recs = [r for r in recs if "," not in r["street"] and re.search(r"\d", r["number"]) and len(r["number"]) <= 12 and len(r["street"]) >= 2 and (r.get("city") or r.get("postcode"))]
    pool = Pool(recs)
    ctx = Ctx(divs, salt)
    def jp_alt(rec):
        m = JP_CHOME.match(rec["street"])
        town = m.group(1) if m else rec["street"]
        tr = divs.get(("JP", town)) or divs.get(("JP", town.removeprefix("大字")))
        if not tr: return None
        st = f"{tr} {kanji_to_int(m.group(2))}-chome" if m and kanji_to_int(m.group(2)) else tr
        city = rec.get("city") or ""
        cr = divs.get(("JP", city))
        if not cr and "市" in city[:-1]:
            a, b = city.split("市", 1)[0] + "市", city.split("市", 1)[1]
            ca, cb = divs.get(("JP", a)), divs.get(("JP", b))
            cr = f"{cb}, {ca}" if ca and cb else None
        if not cr: return None
        a = {"street": st, "city": cr}
        if rec.get("region") and divs.get(("JP", rec["region"])): a["region"] = divs[("JP", rec["region"])]
        return a
    def alts(rec):
        cc = rec["country"]
        if cc == "JP": return jp_alt(rec)
        if script_of(rec["street"]) == "Latn": return None
        st = street_alt.get((cc, rec["street"]))
        if not st: return None
        a = {"street": st}
        if rec.get("city"):
            a["city"] = divs.get((cc, rec["city"]))
            if not a["city"]: return None
        if rec.get("region"):
            a["region"] = divs.get((cc, rec["region"]))
            if not a["region"]: a.pop("region")
        return a
    # split by region
    def split_of(r):
        if r["country"] in HELDOUT_COUNTRIES: return "heldout"
        return "heldout" if h(f"{SEED}:{r['rkey']}") % 2 == 0 else "tune"
    by_country = collections.defaultdict(list)
    for r in recs: by_country[(r["country"], "poi" if r.get("name") else "addr")].append(r)
    os.makedirs(OUT, exist_ok=True)
    outs = {s: open(os.path.join(OUT, f"{s}.jsonl"), "w") for s in ("tune", "heldout")}
    stats = collections.Counter(); regions = collections.defaultdict(set); lic = collections.Counter()
    n = 0
    for (cc, kind), rs in sorted(by_country.items()):
        rng.shuffle(rs)
        cap = CAP_POI if kind == "poi" else CAP_ADDR
        # keep both splits represented: take seeds round-robin over regions
        byr = collections.defaultdict(list)
        for r in rs: byr[r["rkey"]].append(r)
        seeds = []
        while len(seeds) < cap and any(byr.values()):
            for k in list(byr):
                if byr[k]: seeds.append(byr[k].pop())
                if len(seeds) >= cap: break
        for r in seeds:
            s = split_of(r)
            c = make_case(r, pool, ctx, rng, alts, f"R-{n:06d}")
            n += 1
            outs[s].write(json.dumps(c, ensure_ascii=False) + "\n")
            stats[(s, cc, c["script"], c["src"])] += 1; regions[(s, cc)].add(r["rkey"]); lic[(c["src"], c["license"])] += 1
    for f in outs.values(): f.close()
    comp = {"cases": {f"{a}|{b}|{c}|{d}": v for (a, b, c, d), v in stats.items()},
            "regions": {f"{a}|{b}": len(v) for (a, b), v in regions.items()},
            "licenses": {f"{a}|{b}": v for (a, b), v in lic.items()}, "pool_records": len(recs),
            "pool_by_country": dict(collections.Counter(r["country"] for r in recs))}
    json.dump(comp, open(os.path.join(OUT, "composition.json"), "w"), indent=1, ensure_ascii=False)
    print("cases", n, "pool", len(recs), file=sys.stderr)

if __name__ == "__main__":
    main()
