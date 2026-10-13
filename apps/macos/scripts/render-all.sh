#!/bin/bash
# Renders every screen offscreen to PNGs, against a throwaway vornd (temp HOME and data dir).
# usage: scripts/render-all.sh <out-dir>
# env: VORND_BIN, VORN_SESSIOND_BIN (default packages/core/target/ci), BIN_DIR (default .build/release)
set -euo pipefail
here=$(cd "$(dirname "$0")/.." && pwd)
mkdir -p "${1:?give an output directory}"
out=$(cd "$1" && pwd)
bin=${BIN_DIR:-$here/.build/release}
vornd=${VORND_BIN:-$here/../../packages/core/target/ci/vornd}
sessiond=${VORN_SESSIOND_BIN:-$(dirname "$vornd")/vorn-sessiond}
fixtures=$here/Tests/VornWorkflowsTests/Fixtures

tmp=$(mktemp -d "${TMPDIR:-/tmp}/vr.XXXXXX")
data=$tmp/d
mkdir -p "$tmp/home" "$data"
printf "PS1='%%1~ %%# '\n" >"$tmp/home/.zshrc"
port=$((52000 + RANDOM % 8000))
HOME=$tmp/home SHELL=/bin/zsh "$vornd" --data-dir "$data" --port "$port" --host 127.0.0.1 \
  --sessiond "$sessiond" --log-file "$tmp/vornd.log" </dev/null &
pid=$!
# sessiond outlives vornd while a shell is open, so it is stopped by its --home.
trap 'kill $pid 2>/dev/null; wait $pid 2>/dev/null; for p in $(pgrep -f -- "--home $data"); do kill "$p"; done 2>/dev/null; rm -rf "$tmp"' EXIT
for _ in $(seq 100); do [ -s "$data/local-token" ] && [ -s "$data/ws-port" ] && break; sleep 0.1; done
[ -s "$data/local-token" ] || { cat "$tmp/vornd.log" >&2; exit 1; }

"$bin/VornTasksPreview" --data-dir "$data" --seed
# A repo with uncommitted work, so the Changes panel and the files tree have something to show.
repo=$data/projects/vorn
git -C "$repo" init -q -b main
mkdir -p "$repo/src"
printf 'export function greet(name: string) {\n  return `Hello, ${name}`\n}\n' >"$repo/src/greet.ts"
printf '# vorn\n' >"$repo/README.md"
git -C "$repo" add -A
git -C "$repo" -c user.name=t -c user.email=t@t commit -qm init
printf 'export function greet(name: string, excited = false) {\n  const line = `Hello, ${name}`\n  return excited ? `${line}!` : line\n}\n' >"$repo/src/greet.ts"
printf '# vorn\n\nRun many agents side by side.\n' >"$repo/README.md"
printf 'export const version = "0.8.0"\n' >"$repo/src/version.ts"
node "$here/scripts/seed-test-vornd.mjs" "$data" --shell "$data/projects/vorn" --shells 3 >/dev/null
sleep 2

export VORN_DATA_DIR=$data
"$bin/Vorn" --render "$out/shell-main.png"
"$bin/Vorn" --render "$out/shell-project.png" --project vorn
"$bin/Vorn" --render "$out/shell-no-sidebar.png" --no-sidebar
"$bin/Vorn" --render "$out/app-tasks.png" --view tasks
"$bin/Vorn" --render "$out/shell-tabs.png" --layout tabs
"$bin/Vorn" --render "$out/shell-focused.png" --focused 0
"$bin/Vorn" --render "$out/shell-filter-none.png" --filter error
"$bin/Vorn" --render "$out/shell-columns-2.png" --columns 2
"$bin/Vorn" --render "$out/shell-grid-menu.png" --context grid
"$bin/Vorn" --render "$out/shell-card-menu.png" --context card:0
"$bin/Vorn" --render "$out/shell-card-menu-sub.png" --context card:0 --submenu new-with
for overlay in view-options recent shortcuts new-menu sidebar-options add-project edit-project project-menu; do
  "$bin/Vorn" --render "$out/overlay-$overlay.png" --overlay "$overlay"
done
for mode in worktrees sessions sessions-flat; do
  "$bin/Vorn" --render "$out/sidebar-$mode.png" --sidebar-mode "$mode"
done
"$bin/Vorn" --render "$out/app-workflows.png" --view workflows
for page in appearance general updates notifications worktrees agents ssh mcp keys about; do
  "$bin/Vorn" --render "$out/settings-$page.png" --settings "$page"
done
"$bin/Vorn" --render "$out/shell-update-ready.png" --update-status ready:0.8.1
"$bin/Vorn" --render "$out/panes-files.png" --files 0 --claim 0:1
"$bin/Vorn" --render "$out/panes-editor.png" --files 0:src/greet.ts,README.md
"$bin/Vorn" --render "$out/panes-promoted.png" --promote 0:src/greet.ts
"$bin/Vorn" --render "$out/panes-maximized.png" --files 0:src/greet.ts --maximize files
"$bin/Vorn" --render "$out/panes-changes.png" --changes 0
"$bin/Vorn" --render "$out/panes-commit.png" --changes 0 --commit
"$bin/Vorn" --render "$out/component-branch-picker.png" --size 300x320 --component branch-picker
node "$here/scripts/seed-test-vornd.mjs" "$data" --no-runs --review >/dev/null
sleep 3
"$bin/Vorn" --render "$out/app-workflows-review.png" --view workflows --review

for state in list kanban dialog detail options; do
  "$bin/VornTasksPreview" --sample --render "$out/tasks-$state.png" --state "$state"
done
for state in detail-review commit; do
  "$bin/VornTasksPreview" --sample --render "$out/tasks-$state.png" --state "$state" --size 1200x1500
done
"$bin/VornTasksPreview" --data-dir "$data" --render "$out/tasks-vornd.png"

for scene in landing review failed page toast menu; do
  "$bin/WorkflowsPreview" --fixtures "$fixtures" --render "$out/workflows-$scene.png" --scene "$scene"
done
"$bin/WorkflowsPreview" --data-dir "$data" --render "$out/workflows-vornd.png"
"$bin/WorkflowsPreview" --fixtures "$fixtures" --render "$out/workflows-run-prompt.png" --scene run-prompt --size 1280x1000
"$bin/WorkflowsPreview" --fixtures "$fixtures" --render "$out/workflows-missed.png" --scene missed
ls "$out"
