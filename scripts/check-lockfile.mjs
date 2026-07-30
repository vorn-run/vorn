#!/usr/bin/env node
// Guards yarn.lock against registry leakage.
//
// Public npm is unreachable on some machines (corporate proxies), so installs
// run against a mirror. When the mirror serves tarballs from a different host
// than the registry it advertises, Yarn pins that host into the locator:
//
//   resolution: "@scope/pkg@npm:2.0.11::__archiveUrl=https%3A%2F%2Finternal..."
//
// Committing that leaks an internal endpoint into a public repo and pins CI to
// a host it cannot reach. The archive URL is redundant — the checksum already
// identifies the tarball — so stripping it is safe and keeps `--immutable`
// happy on the public registry.
//
// Usage: node scripts/check-lockfile.mjs [--fix]

import { readFileSync, writeFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { dirname, join } from 'node:path'

const LOCKFILE = join(dirname(fileURLToPath(import.meta.url)), '..', 'yarn.lock')

// Hosts a lockfile in this repo may legitimately reference. Anything else is
// assumed to be a private mirror that other clones and CI cannot resolve.
const ALLOWED_HOSTS = new Set([
  'registry.npmjs.org',
  'registry.yarnpkg.com',
  'github.com',
  'codeload.github.com',
  'raw.githubusercontent.com'
])

const fix = process.argv.includes('--fix')
const original = readFileSync(LOCKFILE, 'utf8')

// Yarn locators encode bindings as `base::key=value&key2=value2`. Operate on
// the quoted locator itself, so dropping the trailing binding cannot swallow
// the closing quote, and sibling bindings survive.
const stripBindingFromLocator = (locator) => {
  const [base, ...bindingParts] = locator.split('::')
  if (bindingParts.length === 0) return locator

  const kept = bindingParts
    .join('::')
    .split('&')
    .filter((binding) => !binding.startsWith('__archiveUrl='))

  return kept.length > 0 ? `${base}::${kept.join('&')}` : base
}

const stripArchiveUrl = (line) =>
  line.replace(/"([^"]*)"/g, (match, locator) =>
    locator.includes('__archiveUrl=') ? `"${stripBindingFromLocator(locator)}"` : match
  )

let archiveUrlCount = 0
const fixed = original
  .split('\n')
  .map((line) => {
    if (!line.includes('__archiveUrl=')) return line
    archiveUrlCount += 1
    return stripArchiveUrl(line)
  })
  .join('\n')

if (archiveUrlCount > 0 && fix) {
  writeFileSync(LOCKFILE, fixed)
}

const errors = []

if (archiveUrlCount > 0 && !fix) {
  errors.push(
    `${archiveUrlCount} __archiveUrl binding(s) in yarn.lock pin tarballs to a private mirror.\n` +
      `    Run 'yarn lint:lockfile --fix' (or use 'yarn deps:install') before committing.`
  )
}

// Percent-encoded URLs hide inside locators, so normalise before scanning.
const decoded = (fix ? fixed : original).replace(/%3A/gi, ':').replace(/%2F/gi, '/')
for (const [, host] of decoded.matchAll(/https?:\/\/([^\s"/,&]+)/g)) {
  const bareHost = host.replace(/:\d+$/, '').toLowerCase()
  if (!ALLOWED_HOSTS.has(bareHost)) {
    errors.push(
      `yarn.lock references non-public host '${bareHost}'.\n` +
        `    Regenerate the lockfile against the public npm registry, or add the host to\n` +
        `    ALLOWED_HOSTS in scripts/check-lockfile.mjs if it is genuinely public.`
    )
    break
  }
}

if (errors.length > 0) {
  console.error('✗ yarn.lock validation failed:\n')
  for (const error of errors) console.error(`  - ${error}\n`)
  process.exit(1)
}

if (archiveUrlCount > 0) {
  console.log(`✓ yarn.lock: stripped ${archiveUrlCount} private-mirror __archiveUrl binding(s)`)
} else {
  console.log('✓ yarn.lock: no private registry references')
}
