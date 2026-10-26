# A bench service: each call asks Vorn for its sessions, as a sidebar would.
import json
import sys


def send(message):
    message["jsonrpc"] = "2.0"
    sys.stdout.write(json.dumps(message) + "\n")
    sys.stdout.flush()


waiting = {}
asked = heard = 0
while True:
    line = sys.stdin.readline()
    if not line:
        break
    m = json.loads(line)
    method = m.get("method")
    if method is None:
        caller = waiting.pop(m.get("id"), None)
        if caller is None:
            continue
        if "error" in m:
            send({"id": caller, "error": m["error"]})
        else:
            size = len(json.dumps(m.get("result")))
            send({"id": caller, "result": {"bytes": size, "heard": heard}})
    elif method == "guest.init":
        send({"id": m["id"], "result": None})
    elif method == "guest.on-event":
        heard += len(m["params"]["batch"])
    elif method == "guest.handle":
        asked += 1
        rid = f"r{asked}"
        waiting[rid] = m["id"]
        send({"id": rid, "method": "host.call", "params": {"method": "sessions.list", "args": {}}})
    elif "id" in m:
        send({"id": m["id"], "error": {"code": -32601, "message": f"no {method}"}})
