#!/usr/bin/env bash
# The current branch's pull request checks, one row per check.
set -uo pipefail
req=$(cat)
case "$req" in
  *'"method":"rows"'*) ;;
  *) echo "unknown method: $req" >&2; exit 2 ;;
esac
cd "$VORN_PROJECT" || exit 1
err=$(mktemp)
trap 'rm -f "$err"' EXIT
# Exit 8 means some checks are pending; the JSON is still there.
out=$(gh pr checks --json name,workflow,bucket,link --jq '
  map({ id: ((.workflow // "") + "/" + .name), name, workflow: (.workflow // ""),
        state: .bucket, link: (.link // "") })
  | sort_by([({fail: 0, pending: 1, cancel: 2, pass: 3, skipping: 4}[.state]), .id])' 2>"$err")
if [ -z "$out" ]; then
  if grep -qE "no pull requests found|no checks reported" "$err"; then echo '[]'; exit 0; fi
  cat "$err" >&2
  exit 1
fi
echo "$out"
