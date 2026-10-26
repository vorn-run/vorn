#!/usr/bin/env bash
# A bench one-shot: one request in, one answer out; it is always asked with id 1.
read -r request
printf '{"jsonrpc":"2.0","id":1,"result":{"ok":true}}\n'
