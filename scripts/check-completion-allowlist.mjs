#!/usr/bin/env node
/**
 * Verify the committed completion index was generated from the committed
 * allowlist.
 *
 * The generator needs the completion-spec corpus, which is not a dependency,
 * so on most machines it skips and the no-diff check passes trivially. This
 * compares the allowlist's hash against the one recorded at generation time,
 * which catches the case that check cannot: someone adds a command and
 * commits without regenerating.
 */

import crypto from 'node:crypto'
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const HERE = path.dirname(fileURLToPath(import.meta.url))
const ALLOWLIST = path.join(HERE, 'completion-allowlist.txt')
const META = path.join(HERE, '..', 'src', 'renderer', 'lib', 'completion-index', 'meta.json')

const meta = JSON.parse(fs.readFileSync(META, 'utf8'))
const actual = crypto.createHash('sha256').update(fs.readFileSync(ALLOWLIST)).digest('hex')

if (meta.allowlistSha256 !== actual) {
  console.error(
    'The completion allowlist has changed since the index was generated.\n' +
      `  allowlist: ${actual}\n` +
      `  index was built from: ${meta.allowlistSha256}\n` +
      'Install the completion-spec corpus and run `yarn gen:completions`.'
  )
  process.exit(1)
}

console.log(`Completion index matches the allowlist (${meta.commandCount} commands).`)
