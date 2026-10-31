"""The last `n` returns of the history (ModelFlow protocol: JSON on stdin -> JSON on stdout):
the current sample compared with the calibration sample by the PSI step."""
import json, sys

req = json.load(sys.stdin)
n = int((req.get("params") or {}).get("n", 250))
json.dump(req["inputs"]["returns"][-n:], sys.stdout)
