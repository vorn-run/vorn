# Workflow engine bench

`vorn-workflow-bench` drives the workflow engine in a test vornd, started on its
own temporary HOME and `--data-dir`, over the same WebSocket clients use:

- **flows**: 1, 10 and 100 manual workflows of 10 and 100 instant steps,
  triggered at once. Trigger to first step (from `startedAt`), the first step
  as a viewer sees it, cost per step, vornd CPU per step, steps/s and peak
  memory per run.
- **gates**: 1, 10 and 100 runs parked at an approval, then approved one by
  one. Trigger to the gate and approve to the next step.
- **schedule**: 12 workflows due two seconds apart. How many fired and how late.

```sh
BEFORE_BIN=/path/to/older/build run.sh results/$(date +%F)-$(git rev-parse --short=8 HEAD)
```

`BEFORE_BIN` holds another build's `vornd` and `vorn-sessiond`. The two take
turns case by case, so both see the same load on a shared machine. `raw.jsonl`
keeps every row; `summary.md` is the median of the repetitions (lowest–highest)
and the after/before ratio.
