#!/usr/bin/env node
/**
 * Generate the intent bar's completion index from the third-party
 * completion-spec corpus.
 *
 * Output is committed, so builds and tests need neither the corpus nor
 * network access. Regenerate with `yarn gen:completions`; CI checks the
 * result is unchanged.
 *
 * One file per command: completing a command name needs only names.json,
 * and walking an outline loads exactly the one command being typed. Grouping
 * by first letter would mean loading gcloud to complete git.
 */

import crypto from 'node:crypto'
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'
import {
  assertBudget,
  extractSpec,
  resetTruncations,
  truncations
} from './lib/completion-extract.mjs'

const HERE = path.dirname(fileURLToPath(import.meta.url))
const ROOT = path.join(HERE, '..')
const CORPUS = path.join(ROOT, 'node_modules', '@withfig', 'autocomplete')
const OUT_DIR = path.join(ROOT, 'src', 'renderer', 'lib', 'completion-index')
const ALLOWLIST = path.join(HERE, 'completion-allowlist.txt')

// Per-command is the budget that matters: outlines load one at a time, so
// this is what a single completion actually costs. The total is a runaway
// guard for the allowlist, not a page-weight limit.
const NAMES_BUDGET = 40 * 1024
const COMMAND_BUDGET = 96 * 1024
const TOTAL_BUDGET = 1536 * 1024

/** Command names map to filenames, so refuse anything path-shaped. */
const SAFE_NAME = /^[A-Za-z0-9][A-Za-z0-9._-]*$/

function readAllowlist() {
  return fs
    .readFileSync(ALLOWLIST, 'utf8')
    .split('\n')
    .map((line) => line.trim())
    .filter((line) => line && !line.startsWith('#'))
}

async function main() {
  resetTruncations()
  if (!fs.existsSync(CORPUS)) {
    // The corpus is not a dependency: it is large, build-time only, and the
    // index it produces is committed. Regenerating is a deliberate local act.
    console.log(
      'Completion-spec corpus not installed — leaving the committed index as is.\n' +
        'To regenerate: yarn dlx --package @withfig/autocomplete ... or install it\n' +
        `into node_modules and re-run. Expected at ${CORPUS}`
    )
    return
  }

  const corpusPkg = JSON.parse(fs.readFileSync(path.join(CORPUS, 'package.json'), 'utf8'))
  const allowlist = readAllowlist()

  const names = {}
  const outlines = {}
  const missing = []

  for (const command of allowlist) {
    const specPath = path.join(CORPUS, 'build', `${command}.js`)
    if (!fs.existsSync(specPath)) {
      missing.push(command)
      continue
    }
    let spec
    try {
      spec = (await import(pathToFileURL(specPath).href)).default
    } catch (err) {
      missing.push(`${command} (import failed: ${err.message})`)
      continue
    }
    const extracted = extractSpec(spec)
    if (!extracted) {
      missing.push(`${command} (nothing static to index)`)
      continue
    }
    if (!SAFE_NAME.test(extracted.name)) {
      missing.push(`${command} (unsafe spec name "${extracted.name}")`)
      continue
    }
    // null, not undefined: JSON.stringify drops undefined values, which
    // would silently remove commands that have no top-level description.
    names[extracted.name] = extracted.outline.detail ?? null
    outlines[extracted.name] = extracted.outline
  }

  fs.rmSync(path.join(OUT_DIR, 'outlines'), { recursive: true, force: true })
  fs.mkdirSync(path.join(OUT_DIR, 'outlines'), { recursive: true })

  const namesJson = `${JSON.stringify(names, null, 0)}\n`
  assertBudget('names.json', Buffer.byteLength(namesJson), NAMES_BUDGET)
  fs.writeFileSync(path.join(OUT_DIR, 'names.json'), namesJson)

  let total = Buffer.byteLength(namesJson)
  let largest = { name: '', bytes: 0 }
  for (const [name, outline] of Object.entries(outlines)) {
    const json = `${JSON.stringify(outline, null, 0)}\n`
    const bytes = Buffer.byteLength(json)
    assertBudget(`outlines/${name}.json`, bytes, COMMAND_BUDGET)
    if (bytes > largest.bytes) largest = { name, bytes }
    total += bytes
    fs.writeFileSync(path.join(OUT_DIR, 'outlines', `${name}.json`), json)
  }
  assertBudget('completion index', total, TOTAL_BUDGET)

  fs.writeFileSync(
    path.join(OUT_DIR, 'meta.json'),
    `${JSON.stringify(
      {
        source: corpusPkg.name,
        version: corpusPkg.version,
        spdx: 'MIT',
        url: 'https://github.com/withfig/autocomplete',
        commandCount: Object.keys(names).length,
        // Lets CI detect an allowlist edited without regenerating, even on a
        // machine where the corpus is not installed.
        allowlistSha256: crypto
          .createHash('sha256')
          .update(fs.readFileSync(ALLOWLIST))
          .digest('hex'),
        commands: Object.keys(names).sort()
      },
      null,
      2
    )}\n`
  )

  fs.copyFileSync(path.join(CORPUS, 'LICENSE'), path.join(OUT_DIR, 'LICENSE'))

  console.log(
    `Indexed ${Object.keys(names).length} commands (${(total / 1024).toFixed(1)} KB total, ` +
      `largest ${largest.name} at ${(largest.bytes / 1024).toFixed(1)} KB).`
  )
  // Never silently drop anything — a typo in the allowlist would otherwise
  // look like the command simply has no completions.
  if (missing.length) {
    console.log(`Skipped ${missing.length}:`)
    for (const m of missing) console.log(`  - ${m}`)
  }
  if (truncations.length) {
    console.log(`Truncated ${truncations.length}:`)
    for (const t of truncations) console.log(`  - ${t}`)
  }
}

main().catch((err) => {
  console.error(err)
  process.exit(1)
})
