import json, os, glob, datetime, sys, subprocess
from collections import defaultdict

home = os.path.expanduser("~")
logs_root = os.path.join(home, ".workbuddy", "logs")
today = datetime.date.today()
paths = []
for d in (today, today - datetime.timedelta(days=1)):
    paths += glob.glob(os.path.join(logs_root, d.isoformat(), "*.log"))

turns = defaultdict(lambda: {"calls": [], "conv": None, "first_start": None, "last_end": None})
for p in paths:
    try:
        with open(p, "r", encoding="utf-8", errors="ignore") as f:
            for line in f:
                if "qua.end: {" not in line and "qua.start: {" not in line:
                    continue
                is_end = "qua.end: {" in line
                idx = line.find("qua.end: {" if is_end else "qua.start: {")
                body = line[idx + (9 if is_end else 11):].strip()
                try:
                    v = json.loads(body)
                except Exception:
                    continue
                if v.get("request_type") != "chat":
                    continue
                ts = line[1:line.find("]")]
                t = datetime.datetime.strptime(ts, "%Y/%m/%d %H:%M:%S.%f").timestamp() * 1000
                req = v["conversation_request_id"]
                T = turns[req]
                T["conv"] = v["conversation_id"]
                if not is_end:
                    if T["first_start"] is None or t < T["first_start"]:
                        T["first_start"] = t
                else:
                    T["last_end"] = max(T["last_end"] or t, t)
                    T["calls"].append({
                        "prompt": int(v.get("prompt_tokens") or 0),
                        "completion": int(v.get("completion_tokens") or 0),
                        "ttft": float(v.get("ttft_ms") or 0),
                        "total": float(v.get("total_cost_ms") or 0),
                    })
    except Exception:
        continue

latest_req = max(turns.items(), key=lambda kv: kv[1]["last_end"] or 0)[0]
conv = turns[latest_req]["conv"]
conv_turns = [v for v in turns.values() if v["conv"] == conv]
cur = turns[latest_req]

def agg(calls):
    inp = sum(c["prompt"] for c in calls)
    out = sum(c["completion"] for c in calls)
    gen = sum(max(0.0, c["total"] - c["ttft"]) for c in calls)
    return inp, out, (out / (gen / 1000.0) if gen else 0)

inp, out, tps = agg(cur["calls"])
all_calls = [c for t in conv_turns for c in t["calls"]]
tinp, tout, ttps = agg(all_calls)
dur = (cur["last_end"] - cur["first_start"]) if cur["first_start"] else 0
tdur = sum(max(0, (t["last_end"] or 0) - (t["first_start"] or 0)) for t in conv_turns)

print(json.dumps({
    "conv": conv,
    "turns": len(conv_turns),
    "turn": {"calls": len(cur["calls"]), "input": inp, "output": out, "tps": round(tps, 1),
             "ttft": cur["calls"][0]["ttft"], "duration": round(dur)},
    "total": {"calls": len(all_calls), "input": tinp, "output": tout, "tps": round(ttps, 1),
              "duration": round(tdur)},
}, ensure_ascii=False))
