# A bench one-shot: one request in, one answer out.
import json
import sys

request = json.loads(sys.stdin.readline())
answer = {"jsonrpc": "2.0", "id": request["id"], "result": {"method": request["params"]["method"]}}
sys.stdout.write(json.dumps(answer) + "\n")
