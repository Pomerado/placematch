"""osm_extract.py <pbf> <name> <cc or 'gcc'> <outdir>
Reads one Geofabrik extract and writes street-clustered real address records (OpenStreetMap, ODbL)."""
import sys, json, random, collections, os
import osmium

pbf, name, cc_arg, outdir = sys.argv[1:5]
GCC = [("KW", 46.5, 48.5, 28.5, 30.1), ("QA", 50.7, 51.7, 24.4, 26.2), ("AE", 51.0, 56.5, 22.5, 26.5), ("BH", 50.3, 50.9, 25.7, 26.4),
       ("OM", 52.0, 60.0, 16.5, 26.5), ("SA", 34.0, 56.0, 16.0, 32.5)]
NAME_KEYS = ("name:en", "int_name", "name:ru", "name:uk", "name:ja", "name:ja_rm", "name:ja-Latn", "name:ko", "name:ko-Latn",
             "name:th", "name:th-Latn", "name:zh", "name:zh-Latn", "name:ar", "name:ar-Latn", "name:he", "name:el", "name:bg", "name:sr", "name:ka", "name:hy", "name:hi")
POI_KEYS = ("shop", "amenity", "office", "tourism", "craft", "leisure", "healthcare")

def gcc_cc(lon, lat):
    for c, x0, x1, y0, y1 in GCC:
        if x0 <= lon <= x1 and y0 <= lat <= y1: return c
    return None

class H(osmium.SimpleHandler):
    def __init__(self):
        super().__init__()
        self.groups = collections.defaultdict(list)
        self.pois = []; self.npoi = 0
        self.streets = {}
        self.rng = random.Random(5)
        self.n = 0
    def rec(self, o, kind, cc):
        t = o.tags
        hn = t.get("addr:housenumber")
        if hn:
            street = t.get("addr:street"); place = t.get("addr:place")
            if street or place:
                d = {"src": "osm", "license": "ODbL-1.0", "attribution": "© OpenStreetMap contributors", "country": cc, "city_query": name,
                     "tile": name, "osm_id": f"{kind}{o.id}", "number": hn, "street": street, "place": place,
                     "unit": t.get("addr:unit") or t.get("addr:flats") or t.get("addr:door"), "floor": t.get("addr:floor"),
                     "postcode": t.get("addr:postcode"), "city": t.get("addr:city"),
                     "district": t.get("addr:suburb") or t.get("addr:district") or t.get("addr:quarter") or t.get("addr:neighbourhood"),
                     "province": t.get("addr:province") or t.get("addr:state"), "full": t.get("addr:full"), "block": t.get("addr:block_number"),
                     "name": t.get("name"), "name_en": t.get("name:en"), "extra": {k: v for k, v in t if k.startswith("addr:") and ":" in k[5:]}}
                cat = next((f"{k}={t.get(k)}" for k in POI_KEYS if k in t), None)
                self.n += 1
                if cat and t.get("name") and street:
                    d2 = dict(d, kind="poi", category=cat)
                    self.npoi += 1
                    if len(self.pois) < 3000: self.pois.append(d2)
                    else:
                        j = self.rng.randrange(self.npoi)
                        if j < 3000: self.pois[j] = d2
                if len(self.groups) < 60000 or (cc, d["city"], street or place) in self.groups:
                    g = self.groups[(cc, d["city"], street or place)]
                    if len(g) < 20: g.append(dict(d, kind="address", name=None))
        if "highway" in t and "name" in t and kind == "w":
            names = {k[5:] if k.startswith("name:") else k: t.get(k) for k in NAME_KEYS if k in t}
            if names.get("en") or len(names) > 0:
                self.streets.setdefault((cc, t.get("name")), {"country": cc, "city_query": name, "tile": name, "osm_id": f"w{o.id}", "name": t.get("name"), "names": names})
    def node(self, n):
        if cc_arg == "gcc":
            if "addr:housenumber" not in n.tags: return
            cc = gcc_cc(n.location.lon, n.location.lat)
            if not cc: return
        else: cc = cc_arg
        self.rec(n, "n", cc)
    def way(self, w):
        if cc_arg == "gcc":
            cc = w.tags.get("addr:country")
            if not cc and "highway" in w.tags: cc = "SA"
            if not cc: return
        else: cc = cc_arg
        self.rec(w, "w", cc)

h = H()
h.apply_file(pbf)
rng = random.Random(9)
by_cc = collections.defaultdict(list)
for k, g in h.groups.items(): by_cc[k[0]].append(g)
os.makedirs(outdir, exist_ok=True)
with open(os.path.join(outdir, "addresses.jsonl"), "a") as out:
    for cc, gs in by_cc.items():
        rng.shuffle(gs); kept = 0
        for g in gs:
            rng.shuffle(g)
            for d in g[:15]:
                out.write(json.dumps(d, ensure_ascii=False) + "\n"); kept += 1
            if kept >= 40000: break
        print(name, cc, "addr_kept", kept, "groups", len(gs), file=sys.stderr)
with open(os.path.join(outdir, "pois.jsonl"), "a") as out:
    for d in h.pois: out.write(json.dumps(d, ensure_ascii=False) + "\n")
with open(os.path.join(outdir, "streets.jsonl"), "a") as out:
    for d in h.streets.values():
        if d["names"].get("en"): out.write(json.dumps(d, ensure_ascii=False) + "\n")
print(name, "seen", h.n, "pois", len(h.pois), "streets_en", sum(1 for d in h.streets.values() if d["names"].get("en")), file=sys.stderr)
