import duckdb, json, sys, re, urllib.request
REL = "2026-10-21.0"
base = "https://overturemaps-us-west-2.s3.us-west-2.amazonaws.com"
keys = []
xml = urllib.request.urlopen(f"{base}/?list-type=2&prefix=release/{REL}/theme=addresses/&max-keys=1000", timeout=60).read().decode()
keys = re.findall(r"<Key>([^<]*parquet)</Key>", xml)
con = duckdb.connect()
con.execute("INSTALL httpfs; LOAD httpfs; SET s3_region='us-west-2';")
out = []
for k in keys:
    f = f"s3://overturemaps-us-west-2/{k}"
    rows = con.execute(f"SELECT row_group_id, row_group_num_rows, stats_min_value, stats_max_value FROM parquet_metadata('{f}') WHERE path_in_schema='country'").fetchall()
    for r in rows:
        out.append({"file": k, "rg": r[0], "n": r[1], "min": r[2], "max": r[3]})
    print(k[-40:], len(rows), file=sys.stderr)
json.dump(out, open(sys.argv[1], "w"))
