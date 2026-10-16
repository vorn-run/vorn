#!/usr/bin/env node
// Fails when a test run left a process behind: a vornd, vorn-sessiond, vorn,
// recovery program or Tailscale CLI whose command line or environment names
// one of the given paths (a test's temporary root, the core's target dir).
// Processes that name none of them, such as a running app's, are not looked at.
//
// usage: node scripts/check-leftover-processes.mjs [--wait SECONDS] [--kill] PATH...

import { endAll, fold, leftAfter, leftovers, spellings } from './lib/leftover-processes.mjs'

function parseArgs(argv) {
  const opts = { wait: 10, kill: false, roots: [] }
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i]
    if (a === '--wait') opts.wait = Number(argv[++i])
    else if (a === '--kill') opts.kill = true
    else opts.roots.push(a)
  }
  if (opts.roots.length === 0 || !Number.isFinite(opts.wait)) {
    console.error('usage: check-leftover-processes.mjs [--wait SECONDS] [--kill] PATH...')
    process.exit(2)
  }
  return opts
}

const opts = parseArgs(process.argv.slice(2))
const roots = spellings(opts.roots).map(fold)
// Processes told to end with the test run take a moment to notice it ended.
const left = await leftAfter(() => leftovers(roots), opts.wait)
if (left.length === 0) {
  console.log(`No processes left under ${roots.join(', ')}`)
  process.exit(0)
}
console.error(`${left.length} processes outlived the tests:`)
for (const p of left) console.error(`  ${p.pid} ${p.label}`)
if (opts.kill) await endAll(left)
process.exit(1)
