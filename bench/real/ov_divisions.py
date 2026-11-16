import duckdb, json, sys
con = duckdb.connect(); con.execute("INSTALL httpfs; LOAD httpfs; SET s3_region='us-west-2'; SET threads=4; SET memory_limit='6GB';")
f = "s3://overturemaps-us-west-2/release/2026-10-21.0/theme=divisions/type=division/part-00000-fc5242dc-0596-5fd7-b815-8ff9633b67f3-c000.zstd.parquet"
q = f"""SELECT country, subtype, names.primary AS p, map_entries(names.common) AS c
FROM read_parquet('{f}') WHERE subtype IN ('region','county','localadmin','locality','borough','macrohood','neighborhood') AND names.common IS NOT NULL"""
n = 0
with open(sys.argv[1], "w") as out:
    for row in con.execute(q).fetchall():
        common = {e["key"]: e["value"] for e in (row[3] or [])}
        keep = {k: v for k, v in common.items() if k in ("en", "ja-Latn", "ja", "ko-Latn", "ko", "th", "th-Latn", "ru", "uk", "ar", "he", "el", "zh", "zh-Hant", "zh-Latn", "hi", "bg", "sr", "ka", "hy", "fr", "de", "es", "it", "pt", "ru-Latn", "ar-Latn", "el-Latn", "he-Latn", "uk-Latn")}
        if not keep: continue
        out.write(json.dumps({"country": row[0], "subtype": row[1], "primary": row[2], "names": keep}, ensure_ascii=False) + "\n"); n += 1
print("rows", n, file=sys.stderr)
