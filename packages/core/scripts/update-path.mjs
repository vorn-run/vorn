#!/usr/bin/env node
// Proves an installed Vorn updates to the build in --dir through `Vorn update` and a feed served on 127.0.0.1.
import { execFileSync, spawn, spawnSync } from 'node:child_process'
import {
  chmodSync,
  copyFileSync,
  existsSync,
  mkdirSync,
  readdirSync,
  readFileSync,
  rmSync,
  statSync,
  writeFileSync
} from 'node:fs'
import { createServer } from 'node:http'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const USAGE = `usage: update-path.mjs --dir DIR --key KEY --work DIR [--repo OWNER/NAME] [--old FILE]

  --dir   this build's assets and its vorn-update.json
  --key   the feed key to test: mac-arm64, mac-x64, win-x64 or linux-x64
  --work  an empty scratch folder; the installs, HOME and data dir go here
  --repo  where the previous release is (default: GITHUB_REPOSITORY, else vorn-run/vorn)
  --old   start from this zip, installer or AppImage instead of the previous release

The old app is the newest published release before this build. With none, or
one older than the update command, this build updates onto itself under a
higher version: the same steps, with only the version check unproven.`

const CI = process.env.GITHUB_ACTIONS === 'true'
const WIN = process.platform === 'win32'
const MAC = process.platform === 'darwin'

export class Failure extends Error {}

function log(line) {
  console.log(`update-path: ${line}`)
}

function notice(line) {
  console.log(CI ? `::notice title=Update path::${line}` : `update-path: ${line}`)
}

function fail(line) {
  throw new Failure(line)
}

/** A semver version's parts, or null. */
export function parseVersion(v) {
  const m = /^v?(\d+)\.(\d+)\.(\d+)(?:-([0-9A-Za-z.-]+))?(?:\+.*)?$/.exec(v)
  if (!m) return null
  return { core: [+m[1], +m[2], +m[3]], pre: m[4] ? m[4].split('.') : [] }
}

/** Semver precedence: negative when a is older than b. */
export function compareVersions(a, b) {
  const x = parseVersion(a)
  const y = parseVersion(b)
  if (!x || !y) throw new Error(`not a version: ${x ? b : a}`)
  for (let i = 0; i < 3; i++) if (x.core[i] !== y.core[i]) return x.core[i] - y.core[i]
  if (!x.pre.length || !y.pre.length) return y.pre.length - x.pre.length
  for (let i = 0; i < Math.min(x.pre.length, y.pre.length); i++) {
    const [p, q] = [x.pre[i], y.pre[i]]
    if (p === q) continue
    const [pn, qn] = [/^\d+$/.test(p), /^\d+$/.test(q)]
    if (pn && qn) return +p - +q
    if (pn !== qn) return pn ? -1 : 1
    return p < q ? -1 : 1
  }
  return x.pre.length - y.pre.length
}

/** A version above `v` for a rehearsal: the next patch, which outranks any prerelease of it. */
export function nextPatch(v) {
  const p = parseVersion(v)
  if (!p) throw new Error(`not a version: ${v}`)
  return `${p.core[0]}.${p.core[1]}.${p.core[2] + 1}`
}

/** From GitHub's releases, the newest published one older than `version`, or null. */
export function previousRelease(releases, version) {
  return (
    releases
      .filter((r) => !r.draft && parseVersion(r.tag_name))
      .filter((r) => compareVersions(r.tag_name, version) < 0)
      .sort((a, b) => compareVersions(b.tag_name, a.tag_name))[0] ?? null
  )
}

/** The feed as served: every asset from `base`, under `version`. */
export function servedFeed(feed, base, version) {
  return {
    ...feed,
    version,
    assets: feed.assets.map((a) => ({ ...a, url: `${base}/${encodeURIComponent(a.name)}` }))
  }
}

function parseArgs(argv) {
  const args = {}
  for (let i = 0; i < argv.length; i += 2) {
    if (!argv[i].startsWith('--') || argv[i + 1] === undefined) {
      console.error(USAGE)
      process.exit(2)
    }
    args[argv[i].slice(2)] = argv[i + 1]
  }
  if (!args.dir || !args.key || !args.work) {
    console.error(USAGE)
    process.exit(2)
  }
  return args
}

async function github(url, token) {
  const headers = { 'User-Agent': 'vorn-update-path', Accept: 'application/vnd.github+json' }
  if (token) headers.Authorization = `Bearer ${token}`
  const res = await fetch(url, { headers })
  if (!res.ok) fail(`${url} answered HTTP ${res.status}`)
  return res
}

/** The previous release's asset for `key` on disk, or null with the reason. */
async function fetchPrevious({ repo, version, key, into }) {
  const token = process.env.GH_TOKEN || process.env.GITHUB_TOKEN
  const res = await github(`https://api.github.com/repos/${repo}/releases?per_page=100`, token)
  const prev = previousRelease(await res.json(), version)
  if (!prev) return { reason: `${repo} has no published release before ${version}` }
  const feedAsset = prev.assets.find((a) => a.name === 'vorn-update.json')
  if (!feedAsset)
    return { reason: `${prev.tag_name} is not a native release (it has no update feed)` }
  const feed = await (await github(feedAsset.browser_download_url, token)).json()
  const entry = feed.assets.find((a) => a.key === key)
  if (!entry) return { reason: `${prev.tag_name} has no ${key} asset` }
  mkdirSync(into, { recursive: true })
  const file = path.join(into, path.basename(entry.name))
  const body = Buffer.from(await (await github(entry.url, token)).arrayBuffer())
  if (body.length !== entry.size)
    fail(`${entry.name} is ${body.length} bytes, its feed says ${entry.size}`)
  writeFileSync(file, body)
  log(`downloaded ${prev.tag_name}'s ${entry.name}`)
  return { file, version: feed.version, tag: prev.tag_name }
}

function serve(dir, feedBody) {
  return new Promise((resolve) => {
    const server = createServer((req, res) => {
      const name = decodeURIComponent(new URL(req.url, 'http://x').pathname.slice(1))
      if (name === 'vorn-update.json') {
        res.writeHead(200, { 'Content-Type': 'application/json' })
        return res.end(feedBody())
      }
      const file = path.join(dir, path.basename(name))
      if (path.basename(name) !== name || !existsSync(file)) {
        res.writeHead(404)
        return res.end()
      }
      res.writeHead(200, { 'Content-Length': statSync(file).size })
      res.end(readFileSync(file))
    })
    server.listen(0, '127.0.0.1', () => resolve(server))
  })
}

/** Where an install's pieces are, for the platform this runs on. */
function layout(inst) {
  if (MAC) {
    const app = path.join(inst, 'Vorn.app')
    return { app, exe: path.join(app, 'Contents/MacOS/Vorn'), marker: null }
  }
  if (WIN) {
    return {
      app: inst,
      exe: path.join(inst, 'Vorn.exe'),
      marker: path.join(inst, 'resources', 'web', 'update-path-marker')
    }
  }
  const app = path.join(inst, 'Vorn.AppImage')
  return { app, exe: app, marker: null }
}

function run(cmd, args, opts = {}) {
  const r = spawnSync(cmd, args, { encoding: 'utf8', timeout: 120_000, ...opts })
  return { ok: r.status === 0, out: `${r.stdout ?? ''}${r.stderr ?? ''}`.trim(), error: r.error }
}

/** `run` without blocking, for a child that calls back into this process's server. */
function runAsync(cmd, args, { timeout, ...opts }) {
  return new Promise((resolve) => {
    const child = spawn(cmd, args, { ...opts, stdio: ['ignore', 'pipe', 'pipe'] })
    let out = ''
    child.stdout.on('data', (d) => (out += d))
    child.stderr.on('data', (d) => (out += d))
    const timer = setTimeout(() => child.kill(), timeout)
    child.on('error', (error) => resolve({ ok: false, out, error }))
    child.on('close', (status) => {
      clearTimeout(timer)
      resolve({ ok: status === 0, out: out.trim(), error: status === null ? 'timed out' : null })
    })
  })
}

function install(file, inst) {
  rmSync(inst, { recursive: true, force: true })
  mkdirSync(inst, { recursive: true })
  if (MAC) execFileSync('ditto', ['-x', '-k', file, inst])
  else if (WIN) {
    // /D must come last and unquoted; the temp path has no spaces.
    const r = run(file, ['/S', `/D=${inst}`], { windowsVerbatimArguments: true })
    if (!r.ok) fail(`the installer ${path.basename(file)} failed: ${r.out || r.error}`)
  } else {
    copyFileSync(file, layout(inst).app)
    chmodSync(layout(inst).app, 0o755)
  }
  if (!existsSync(layout(inst).exe)) fail(`${path.basename(file)} installed no ${layout(inst).exe}`)
}

function uninstall(inst) {
  if (WIN) {
    for (const im of ['Vorn.exe', 'vornd.exe', 'vorn-sessiond.exe', 'OpenConsole.exe']) {
      run('taskkill', ['/F', '/T', '/IM', im])
    }
    const un = path.join(inst, 'Uninstall Vorn.exe')
    if (existsSync(un)) run(un, ['/S', `_?=${inst}`], { windowsVerbatimArguments: true })
  }
  rmSync(inst, { recursive: true, force: true })
}

function stopAll(work, data) {
  try {
    const { pid } = JSON.parse(readFileSync(path.join(data, 'ws-port'), 'utf8'))
    process.kill(pid)
  } catch {
    // No vornd answered, or it is gone.
  }
  if (WIN) {
    for (const im of ['Vorn.exe', 'vornd.exe', 'vorn-sessiond.exe', 'OpenConsole.exe']) {
      run('taskkill', ['/F', '/T', '/IM', im])
    }
  } else {
    // This script's own command line names the work dir too.
    const pids = run('pgrep', ['-f', work]).out.split(/\s+/).filter(Boolean).map(Number)
    for (const pid of pids.filter((p) => p !== process.pid && p !== process.ppid)) {
      try {
        process.kill(pid, 'SIGKILL')
      } catch {
        // Already gone.
      }
    }
  }
}

/** What the vornd serving `data` says it is, once it answers. */
async function identity(data, deadline) {
  let last = 'no vornd wrote ws-port'
  while (Date.now() < deadline) {
    try {
      const { port } = JSON.parse(readFileSync(path.join(data, 'ws-port'), 'utf8'))
      return await new Promise((resolve, reject) => {
        const ws = new WebSocket(`ws://127.0.0.1:${port}/ws`)
        const timer = setTimeout(() => {
          ws.close()
          reject(new Error('vornd sent no identity'))
        }, 10_000)
        ws.onmessage = (e) => {
          const msg = JSON.parse(String(e.data))
          if (msg.method !== 'server:identity') return
          clearTimeout(timer)
          ws.close()
          resolve(msg.params)
        }
        ws.onerror = () => {
          clearTimeout(timer)
          reject(new Error(`vornd on port ${port} refused the connection`))
        }
      })
    } catch (e) {
      last = e.message
      await new Promise((r) => setTimeout(r, 1000))
    }
  }
  fail(`the updated app started no vornd on its data dir (${last})`)
}

/** A path's inode, which a swap that replaced it changes. */
function inode(p) {
  return statSync(p, { bigint: true }).ino
}

function tail(file, lines = 40) {
  if (!existsSync(file)) return
  console.log(`--- ${file}`)
  console.log(readFileSync(file, 'utf8').split('\n').slice(-lines).join('\n'))
}

async function main() {
  const args = parseArgs(process.argv.slice(2))
  const dir = path.resolve(args.dir)
  const work = path.resolve(args.work)
  const key = args.key
  const repo = args.repo || process.env.GITHUB_REPOSITORY || 'vorn-run/vorn'
  const feed = JSON.parse(readFileSync(path.join(dir, 'vorn-update.json'), 'utf8'))
  const build = feed.version
  const entry = feed.assets.find((a) => a.key === key)
  if (!entry) fail(`this build's feed has no ${key} asset`)
  const newFile = path.join(dir, entry.name)
  if (!existsSync(newFile)) fail(`${entry.name} is not in ${dir}`)

  rmSync(work, { recursive: true, force: true })
  const inst = path.join(work, 'install')
  const data = path.join(work, 'data')
  const home = path.join(work, 'home')
  const cache = path.join(work, 'cache')
  for (const d of [data, home, cache]) mkdirSync(d, { recursive: true })
  // The relaunched app leaves the vorn command alone, as after the person removed it.
  writeFileSync(path.join(data, 'cli-link.json'), JSON.stringify({ removed: true }))

  let old = args.old
    ? { file: path.resolve(args.old) }
    : await fetchPrevious({ repo, version: build, key, into: path.join(work, 'previous') })
  if (!old.file) {
    notice(`${old.reason}; rehearsing with this build updating onto itself`)
    old = null
  }

  let feedBody = ''
  const server = await serve(dir, () => feedBody)
  const base = `http://127.0.0.1:${server.address().port}`
  const env = {
    ...process.env,
    HOME: home,
    XDG_CACHE_HOME: cache,
    VORN_DATA_DIR: data,
    VORN_CLI_DIR: path.join(work, 'bin'),
    VORN_UPDATE_FEED: `${base}/vorn-update.json`,
    VORN_TAILSCALE: 'none',
    VORN_EXIT_WITH: String(process.pid),
    APPIMAGE_EXTRACT_AND_RUN: '1'
  }
  if (WIN) env.LOCALAPPDATA = cache
  const { app, exe, marker } = layout(inst)

  try {
    if (old) {
      install(old.file, inst)
      const help = run(exe, ['--help'], { env, timeout: 60_000 })
      if (!/^\s+update\s/m.test(help.out)) {
        notice(
          `${old.tag ?? old.file} predates the update command; rehearsing with this build updating onto itself`
        )
        uninstall(inst)
        old = null
      }
    }
    if (!old) install(newFile, inst)
    const served = old ? build : nextPatch(build)
    feedBody = JSON.stringify(servedFeed(feed, base, served))
    if (old?.tag && compareVersions(old.version, build) >= 0) {
      fail(`the old app (${old.version}) is not older than this build (${build})`)
    }
    log(
      old
        ? `updating ${old.tag ?? old.file} to ${build}`
        : `updating ${build} to itself served as ${served}`
    )

    if (marker) writeFileSync(marker, '')
    const before = inode(app)
    const up = await runAsync(exe, ['update', '--data-dir', data], { env, timeout: 300_000 })
    console.log(up.out)
    if (!up.ok) fail(`\`Vorn update\` failed: ${up.out.split('\n').pop() || up.error}`)
    if (!up.out.includes(`Installed Vorn ${served}`)) fail(`\`Vorn update\` installed nothing`)

    const id = await identity(data, Date.now() + 180_000)
    log(`the relaunched app's vornd says ${JSON.stringify(id)}`)
    if (id.appVersion !== build) {
      fail(`the relaunched app is ${id.appVersion}, not ${build}: the update did not take`)
    }
    if (id.buildChannel !== 'packaged')
      fail(`the relaunched app's vornd is a ${id.buildChannel} build, not the packaged one`)

    const version = run(exe, ['--version'], { env, timeout: 60_000 })
    if (version.out.split(/\s+/).pop() !== build) {
      fail(`the installed app reports ${JSON.stringify(version.out)}, not ${build}`)
    }
    if (marker ? existsSync(marker) : inode(app) === before) {
      fail(`the update left the old app in place at ${app}`)
    }
    if (MAC) {
      const sig = run('codesign', ['--verify', '--deep', '--strict', '--verbose=2', app])
      console.log(sig.out)
      if (!sig.ok) fail(`the updated app's signature is not valid: ${sig.out}`)
      const left = readdirSync(inst).filter((n) => n.startsWith('.Vorn-update-'))
      if (left.length) fail(`the update left ${left.join(', ')} beside the app`)
    }
    log(
      `ok: ${old ? (old.tag ?? 'the given app') : 'this build'} updated to ${build} and started its vornd`
    )
  } catch (e) {
    tail(path.join(data, 'server.log'))
    throw e
  } finally {
    server.close()
    stopAll(work, data)
    uninstall(inst)
  }
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main().then(
    () => process.exit(0),
    (e) => {
      const line = e instanceof Failure ? e.message : e.stack
      console.error(CI ? `::error title=Update path::${line}` : `update-path: ${line}`)
      process.exit(1)
    }
  )
}
