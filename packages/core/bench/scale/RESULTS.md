# Scale bench results

How the session holder (`vorn-sessiond`) and vornd hold up with many
terminals. Each tier is N sessions in all: one probe, a tenth printing a
build log, and the rest idle. Every section below is one run of
`scripts/bench-gcp.sh` on a fresh Spot VM; the raw JSON, logs and host
limits sit beside this file under `results/<date>-<commit>/`.

- **holder alone**: the bench starts `vorn-sessiond` and drives it over its
  socket as vornd does. Idle sessions run `cat`, so the numbers are the
  holder's own cost. Handoff starts a second holder in the same home and
  times `Adopt` until every live session has moved.
- **vornd + holder**: the bench starts vornd, which launches its holder.
  Sessions are started on vornd's app channel and followed over the grid
  endpoint, as the app does. Idle sessions run `bash`.
- **per idle terminal** is the RSS growth from no sessions to all idle ones,
  divided by the idle count; **per busy terminal** is the growth once the
  busy ones run, divided by the busy count. **Machine memory per idle
  terminal** is the drop in `MemAvailable` over the same step, so it
  includes the programs and the kernel's pty and thread costs.
- **probe echo** is a keystroke written to the probe until its echo comes
  back: as holder records (holder alone) or as a grid frame (vornd +
  holder).
- **throughput** is 16 sessions each printing 16 MiB as fast as they can,
  all attached to one grid client, timed until the client's screens show
  every log's last line.

Run it with `scripts/bench-gcp.sh [TIER...]`. Tiers above 100 refuse to run
unless `VORN_BENCH_HOST=1` is set, which only the VM's runner does.
