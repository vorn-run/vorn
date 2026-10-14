#!/usr/bin/env bash
# One request in, one answer out: a one-shot is always asked with id 1.
read -r request
name=$(printf '%s' "$request" | sed -n 's/.*"name":"\([^"]*\)".*/\1/p')
printf '{"jsonrpc":"2.0","id":1,"result":{"greeting":"hello %s"}}\n' "${name:-there}"
