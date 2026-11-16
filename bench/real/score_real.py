#!/usr/bin/env python3
"""score_real.py cases.jsonl name=pred.jsonl [name=pred.jsonl ...] [--md out.md] [--json out.json] [--min N]

Per category: accuracy and wrong-pick rate for each predictor. A wrong pick is a `match` answer whose
index is not an accepted listing. Wrong picks are also broken down by the role of the listing picked.
"""
import json, sys, collections
args = sys.argv[1:]
def opt(k, d=None): return args[args.index(k) + 1] if k in args else d
cases = [json.loads(l) for l in open(args[0])]
preds = {}
for a in args[1:]:
    if "=" in a and not a.startswith("--"):
        n, p = a.split("=", 1)
        preds[n] = {json.loads(l)["id"]: json.loads(l) for l in open(p) if l.strip()}
KINDS = ("pos", "pick_match", "pick_none", "amb_city", "unit_added_only")
PERT = ("abbrev", "abbrev_site", "case", "diacritics", "no_commas", "reorder", "reorder_pc", "typo", "translit", "exonym",
        "business", "business_query", "digits_native", "jp_form", "postcode_dropped", "unit_dropped_c", "city_dropped")
def kind(c):
    for t in c["tags"]:
        if t in KINDS: return t
        if t.startswith("neg_"): return t
    return "other"
def judge(c, p):
    out = p["outcome"] if p else "missing"
    if out == "match":
        if c["expected"] == "match" and p.get("index") in c["accept"]: return "ok", None
        roles = c.get("roles") or []
        i = p.get("index")
        role = roles[i] if i is not None and i < len(roles) else "?"
        return "wrong", f"{c['expected']}->picked:{role}"
    return ("ok" if out == c["expected"] else "miss"), None
groups = collections.defaultdict(list)
for c in cases:
    groups["overall"].append(c)
    groups["task:" + c["task"]].append(c)
    groups["kind:" + kind(c)].append(c)
    groups["script:" + c["script"]].append(c)
    groups["src:" + c.get("src", "?")].append(c)
    groups["country:" + c["country"]].append(c)
    for t in c["tags"]:
        if t in PERT: groups["pert:" + t].append(c)
    if not any(t in PERT for t in c["tags"]): groups["pert:(none)"].append(c)
res = {}
wrongcat = {n: collections.Counter() for n in preds}
for g, cs in groups.items():
    row = {"n": len(cs)}
    for n, P in preds.items():
        ok = wp = 0
        for c in cs:
            v, cat = judge(c, P.get(c["id"]))
            ok += v == "ok"; wp += v == "wrong"
            if g == "overall" and cat: wrongcat[n][cat] += 1
        row[n] = (ok / len(cs), wp / len(cs), wp)
    res[g] = row
names = list(preds)
def order(g):
    pre = g.split(":")[0]
    return (["overall", "task", "kind", "pert", "script", "src", "country"].index(pre) if pre in ["overall", "task", "kind", "pert", "script", "src", "country"] else 9, g)
lines = ["| category | n | " + " | ".join(f"{n} acc / wrong" for n in names) + " |", "|---|---|" + "---|" * len(names)]
mn = int(opt("--min", 30))
for g in sorted(res, key=order):
    r = res[g]
    if r["n"] < mn and not g.startswith(("kind", "script")): continue
    lines.append(f"| {g} | {r['n']} | " + " | ".join(f"{100*r[n][0]:.1f} / {100*r[n][1]:.2f}" for n in names) + " |")
lines.append("")
lines.append("Wrong picks by the role of the listing picked:")
for n in names:
    lines.append(f"- {n}: " + ", ".join(f"{k} {v}" for k, v in wrongcat[n].most_common()))
txt = "\n".join(lines)
if opt("--md"): open(opt("--md"), "w").write(txt + "\n")
if opt("--json"): json.dump({"groups": res, "wrong_categories": {n: dict(v) for n, v in wrongcat.items()}}, open(opt("--json"), "w"), indent=1)
print(txt if "--quiet" not in args else "\n".join(l for l in lines if l.startswith(("| overall", "| task", "| kind", "| script", "- ", "Wrong", "| category", "|---"))))
