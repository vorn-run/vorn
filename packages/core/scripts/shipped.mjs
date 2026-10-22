#!/usr/bin/env node
// Fetches the shipped set: the packs shipped.toml pins, into
// packages/core/shipped/, where vornd's build embeds them; and the packs
// pinned.toml pins, which only the catalog offers, into shipped/catalog/,
// where the tests install them from. Each pack is checked against its sha256.
//
//   node scripts/shipped.mjs                    the packs the extensions repo's CI built at the rev (needs gh)
//   node scripts/shipped.mjs --from <checkout>  builds them from a checkout of that repo (on Linux)
//
// A component's Wasm differs by build host, and the extensions release on
// Linux, so the pins are the Linux build's. Until the shipped extensions are
// released, CI and packaging take the packs from that repository's CI run.
import { execFileSync } from 'node:child_process'
import { createHash } from 'node:crypto'
import { copyFileSync, mkdirSync, readFileSync, rmSync } from 'node:fs'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const core = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const REPO = 'vorn-run/extensions'

/** The lock: its rev and each extension's id, path, version and sha256. */
export function readLock(text) {
  const rev = text.match(/^rev\s*=\s*"([^"]+)"/m)?.[1]
  const extensions = text
    .split(/^\[\[extension\]\]\s*$/m)
    .slice(1)
    .map((block) => {
      const field = (key) => block.match(new RegExp(`^${key}\\s*=\\s*"([^"]*)"`, 'm'))?.[1]
      return {
        id: field('id'),
        path: field('path'),
        version: field('version'),
        sha256: field('sha256')
      }
    })
  return { rev, extensions }
}

function run(cmd, args, cwd) {
  execFileSync(cmd, args, { cwd, stdio: 'inherit' })
}

/** Packs the checkout's extensions into its dist/. */
function build(checkout, lock) {
  const head = execFileSync('git', ['rev-parse', 'HEAD'], {
    cwd: checkout,
    encoding: 'utf8'
  }).trim()
  if (head !== lock.rev) {
    console.warn(`shipped: ${checkout} is at ${head}, and shipped.toml pins ${lock.rev}`)
  }
  const paths = [...lock.extensions, ...lock.catalog].map((e) => e.path)
  run('cargo', ['run', '--locked', '--release', '-p', 'vorn-pack', '--', ...paths], checkout)
  return join(checkout, 'dist')
}

/** Downloads the packs the extensions repo's CI built at the rev. */
function download(lock) {
  const dir = join(core, 'target', 'shipped-download')
  rmSync(dir, { recursive: true, force: true })
  const list = ['run', 'list', '-R', REPO, '--workflow', 'ci.yml', '--commit', lock.rev]
  const query = ['--limit', '1', '--json', 'databaseId', '-q', '.[0].databaseId']
  const runId = execFileSync('gh', [...list, ...query], { encoding: 'utf8' }).trim()
  if (!runId) throw new Error(`no CI run of ${REPO} at ${lock.rev} to take the packs from`)
  run('gh', ['run', 'download', runId, '-R', REPO, '-n', 'rust-extensions', '-D', dir], core)
  return dir
}

function main() {
  const args = process.argv.slice(2)
  const at = args.indexOf('--from')
  const lock = readLock(readFileSync(join(core, 'shipped.toml'), 'utf8'))
  lock.catalog = readLock(readFileSync(join(core, 'pinned.toml'), 'utf8')).extensions
  const all = [...lock.extensions, ...lock.catalog]
  if (!lock.rev || all.some((e) => !e.id || !e.version || !e.sha256)) {
    throw new Error('shipped.toml: a rev, and an id, version and sha256 for every extension')
  }
  const packs = at >= 0 && args[at + 1] ? build(resolve(args[at + 1]), lock) : download(lock)
  const out = join(core, 'shipped')
  const wrong = []
  for (const [into, set] of [
    [out, lock.extensions],
    [join(out, 'catalog'), lock.catalog]
  ]) {
    mkdirSync(into, { recursive: true })
    for (const e of set) {
      const name = `${e.id}-${e.version}.vorn.tgz`
      const from = join(packs, name)
      const sha = createHash('sha256').update(readFileSync(from)).digest('hex')
      if (sha !== e.sha256) {
        wrong.push(`${name}: got ${sha}, pinned ${e.sha256}`)
        continue
      }
      copyFileSync(from, join(into, name))
      console.log(`shipped: ${name} ${sha}`)
    }
  }
  if (wrong.length) {
    throw new Error(`not the packs shipped.toml and pinned.toml pin:\n${wrong.join('\n')}`)
  }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    main()
  } catch (err) {
    console.error(`shipped: ${err.message}`)
    process.exit(1)
  }
}
