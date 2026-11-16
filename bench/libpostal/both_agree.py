import json, sys
a = {json.loads(l)["id"]: json.loads(l) for l in open(sys.argv[1]) if l.strip()}
b = {json.loads(l)["id"]: json.loads(l) for l in open(sys.argv[2]) if l.strip()}
with open(sys.argv[3], "w") as fh:
    for i, x in a.items():
        y = b.get(i, {})
        if x["outcome"] == y.get("outcome") and x.get("index") == y.get("index"): o = {"outcome": x["outcome"], "index": x.get("index")}
        elif "none" == x["outcome"] == y.get("outcome"): o = {"outcome": "none", "index": None}
        elif x["outcome"] == "match" and y.get("outcome") == "match": o = {"outcome": "ambiguous", "index": None}
        elif x["outcome"] == "none" or y.get("outcome") == "none": o = {"outcome": "none", "index": None}
        else: o = {"outcome": "ambiguous", "index": None}
        o["id"] = i; fh.write(json.dumps(o) + "\n")
