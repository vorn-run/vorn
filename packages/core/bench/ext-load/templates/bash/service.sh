#!/usr/bin/env bash
# A bench service: answers each request at once, with no process per line.
request='^\{"jsonrpc":"2\.0","id":([0-9]+),"method":"([a-z.-]+)"'
while IFS= read -r line; do
  [[ $line =~ $request ]] || continue
  if [[ ${BASH_REMATCH[2]} == guest.init ]]; then
    printf '{"jsonrpc":"2.0","id":%s,"result":null}\n' "${BASH_REMATCH[1]}"
  else
    printf '{"jsonrpc":"2.0","id":%s,"result":{"ok":true}}\n' "${BASH_REMATCH[1]}"
  fi
done
