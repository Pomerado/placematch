"""Sample real addresses from Overture Maps addresses (OpenAddresses + government sources).
Keeps street clusters so near-neighbour distractors exist. Open licenses only."""
import json, sys, random, collections, os
import pyarrow.fs as pafs, pyarrow.parquet as pq
meta = json.load(open(sys.argv[1])); outp = sys.argv[2]
RGS_PER_COUNTRY = int(os.environ.get("RGS", "8")); STREETS_PER_RG = 220; PER_STREET = 14
OPEN = ("CC0", "CC-BY", "ODbL", "PDDL", "Public-Domain", "OGL", "PD", "dl-de", "etalab", "NLOD", "CC-PDDC", "Open", "MIT", "GSI", "LO-2.0")
fs = pafs.S3FileSystem(anonymous=True, region="us-west-2")
byc = collections.defaultdict(list)
for r in meta:
    if r["min"] == r["max"]: byc[r["min"]].append(r)
done = set()
if os.path.exists(outp):
    for l in open(outp): done.add(json.loads(l)["_rg"])
cols = ["street", "number", "unit", "postcode", "postal_city", "address_levels", "country", "sources"]
rnd = random.Random(7)
lic_stats = collections.Counter()
with open(outp, "a") as out:
    for cc, rgs in sorted(byc.items()):
        k = min(RGS_PER_COUNTRY, len(rgs))
        step = len(rgs) / k
        chosen = [rgs[int(i * step + rnd.random() * step * 0.5)] for i in range(k)]
        for r in chosen:
            key = f"{r['file']}#{r['rg']}"
            if key in done: continue
            try:
                pf = pq.ParquetFile(fs.open_input_file("overturemaps-us-west-2/" + r["file"]))
                t = pf.read_row_group(r["rg"], columns=cols).to_pylist()
            except Exception as e:
                print("ERR", cc, key, e, file=sys.stderr); continue
            ok = []
            for row in t:
                src = (row.get("sources") or [{}])[0] or {}
                lic = src.get("license") or ""
                lic_stats[(cc, lic)] += 1
                if not any(x.lower() in lic.lower() for x in OPEN): continue
                if not row.get("street") or not row.get("number"): continue
                row["_lic"] = lic; row["_dataset"] = src.get("dataset"); ok.append(row)
            streets = collections.defaultdict(list)
            for row in ok: streets[row["street"]].append(row)
            names = list(streets); rnd.shuffle(names)
            n = 0
            for s in names[:STREETS_PER_RG]:
                rows = streets[s]; rnd.shuffle(rows)
                for row in rows[:PER_STREET]:
                    lv = [x["value"] for x in (row.get("address_levels") or []) if x and x.get("value")]
                    rec = {"src": "overture", "dataset": row["_dataset"], "license": row["_lic"], "country": cc,
                           "number": row["number"], "street": row["street"], "unit": row.get("unit"),
                           "postcode": row.get("postcode"), "postal_city": row.get("postal_city"), "levels": lv, "_rg": key}
                    out.write(json.dumps(rec, ensure_ascii=False) + "\n"); n += 1
            print(cc, key[-30:], "rows", len(t), "open", len(ok), "kept", n, file=sys.stderr, flush=True)
json.dump({f"{a}|{b}": v for (a, b), v in lic_stats.items()}, open(outp + ".licenses.json", "w"), ensure_ascii=False, indent=0)
