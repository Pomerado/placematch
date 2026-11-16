import json, sys, time, os
sys.path.insert(0, os.path.dirname(__file__))
t0 = time.time()
import lpmatch
print(f"load {time.time()-t0:.2f}s", file=sys.stderr)
HINT = os.environ.get("HINT", "1") == "1"
def run(c):
    cc = c["country"] if HINT else None
    t = time.perf_counter()
    if c["task"] == "pair":
        v = lpmatch.compare(c["query"], c["candidates"][0], cc)[0]
        out = {"id": c["id"], "outcome": "match" if v == "same" else "none", "index": 0 if v == "same" else None}
    else:
        r = lpmatch.pick(c["query"], c["candidates"], cc)
        out = {"id": c["id"], "outcome": r["outcome"], "index": r["index"]}
    out["us"] = int((time.perf_counter() - t) * 1e6)
    return out
def worker(cases, path):
    with open(path, "a") as fh:
        for c in cases:
            fh.write(json.dumps(run(c)) + "\n"); fh.flush()
    os._exit(0)
if __name__ == "__main__":
    cases = [json.loads(l) for l in open(sys.argv[1])]
    P = int(os.environ.get("P", "8")); outp = sys.argv[2]
    todo = {k: cases[k::P] for k in range(P)}
    crashes = []; sigs = []
    while todo:
        pids = {}
        for k, cs in todo.items():
            path = f"{outp}.part{k}"
            pid = os.fork()
            if pid == 0: worker(cs, path)
            pids[pid] = k
        nxt = {}
        for _ in range(len(pids)):
            pid, st = os.wait(); k = pids[pid]
            if st: sigs.append(st & 0x7f or ("exit", st >> 8))
            done = {json.loads(l)["id"] for l in open(f"{outp}.part{k}")} if os.path.exists(f"{outp}.part{k}") else set()
            rest = [c for c in todo[k] if c["id"] not in done]
            if rest:
                bad = rest[0]; crashes.append(bad["id"])
                with open(f"{outp}.part{k}", "a") as fh:
                    fh.write(json.dumps({"id": bad["id"], "outcome": "none", "index": None, "crash": True, "us": 0}) + "\n")
                if rest[1:]: nxt[k] = rest[1:]
        todo = nxt
    with open(outp, "w") as fh:
        for k in range(P):
            p = f"{outp}.part{k}"
            if os.path.exists(p):
                fh.write(open(p).read()); os.remove(p)
    print(f"done {time.time()-t0:.1f}s crashes={len(crashes)} {crashes[:20]} statuses={sigs[:20]}", file=sys.stderr)
