#!/usr/bin/env node
// Fetches the ConPTY that vorn-sessiond ships with on Windows: conpty.dll and
// the OpenConsole.exe it hosts sessions in, from Microsoft's
// Microsoft.Windows.Console.ConPTY package. The inbox ConPTY lags Windows
// Terminal's by years, and sessions that must survive app updates are better
// off on a host the app chooses.
//
// They land beside ./vorn-sessiond.exe, laid out as the package lays them out
// beside an executable: conpty.dll next to it and OpenConsole.exe in a folder
// named for the architecture. vorn-sessiond loads conpty.dll from its own
// directory, and vornd copies the lot with it when it installs a version.
//
//   --arch x64|arm64   default: this machine's
//   --out <dir>        default: . (packages/core)
//
// The download is pinned by version and checked against its SHA-256, and
// kept under target/ so a rebuild does not fetch it again.
import { createHash } from 'node:crypto'
import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { inflateRawSync } from 'node:zlib'

const VERSION = '1.24.261001001'
const SHA256 = '4d6aaddc1d2385c9f5897df28f33879f699f8f2783315d5204cf3d8c3616ac5f'
const URL = `https://api.nuget.org/v3-flatcontainer/microsoft.windows.console.conpty/${VERSION}/microsoft.windows.console.conpty.${VERSION}.nupkg`

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')

/** The entries of a zip archive, by name: just enough of the format for a nupkg. */
function unzip(buf) {
  // The end of central directory record, searched from the end past any comment.
  let eocd = -1
  for (let i = buf.length - 22; i >= Math.max(0, buf.length - 22 - 0xffff); i--) {
    if (buf.readUInt32LE(i) === 0x06054b50) {
      eocd = i
      break
    }
  }
  if (eocd < 0) throw new Error('not a zip archive')
  const count = buf.readUInt16LE(eocd + 10)
  let at = buf.readUInt32LE(eocd + 16)
  const files = new Map()
  for (let n = 0; n < count; n++) {
    if (buf.readUInt32LE(at) !== 0x02014b50) throw new Error('bad central directory')
    const method = buf.readUInt16LE(at + 10)
    const size = buf.readUInt32LE(at + 20)
    const nameLen = buf.readUInt16LE(at + 28)
    const extraLen = buf.readUInt16LE(at + 30)
    const commentLen = buf.readUInt16LE(at + 32)
    const local = buf.readUInt32LE(at + 42)
    const name = buf.toString('utf8', at + 46, at + 46 + nameLen)
    const dataAt = local + 30 + buf.readUInt16LE(local + 26) + buf.readUInt16LE(local + 28)
    files.set(name, { method, at: dataAt, size: buf.readUInt32LE(at + 24), packed: size })
    at += 46 + nameLen + extraLen + commentLen
  }
  return (name) => {
    const f = files.get(name)
    if (!f) throw new Error(`${name} is not in the package`)
    const raw = buf.subarray(f.at, f.at + f.packed)
    const out = f.method === 0 ? Buffer.from(raw) : inflateRawSync(raw)
    if (out.length !== f.size) throw new Error(`${name}: ${out.length} bytes, expected ${f.size}`)
    return out
  }
}

async function nupkg() {
  const cache = path.join(root, 'target', 'conpty', `${VERSION}.nupkg`)
  const sha = (b) => createHash('sha256').update(b).digest('hex')
  if (existsSync(cache)) {
    const kept = readFileSync(cache)
    if (sha(kept) === SHA256) return kept
  }
  const res = await fetch(URL)
  if (!res.ok) throw new Error(`${URL}: HTTP ${res.status}`)
  const body = Buffer.from(await res.arrayBuffer())
  const got = sha(body)
  if (got !== SHA256) throw new Error(`${URL}: SHA-256 ${got}, expected ${SHA256}`)
  mkdirSync(path.dirname(cache), { recursive: true })
  writeFileSync(cache, body)
  return body
}

/** Writes conpty.dll and <arch>/OpenConsole.exe under `out`, and answers `out`. */
export async function fetchConpty({ arch = process.arch, out = root } = {}) {
  if (arch !== 'x64' && arch !== 'arm64') throw new Error(`no ConPTY for ${arch}`)
  const read = unzip(await nupkg())
  mkdirSync(path.join(out, arch), { recursive: true })
  writeFileSync(path.join(out, 'conpty.dll'), read(`runtimes/win-${arch}/native/conpty.dll`))
  writeFileSync(
    path.join(out, arch, 'OpenConsole.exe'),
    read(`build/native/runtimes/${arch}/OpenConsole.exe`)
  )
  console.log(`ConPTY ${VERSION} (${arch}) in ${path.relative(process.cwd(), out) || '.'}`)
  return out
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const argv = process.argv.slice(2)
  const opt = (name) => {
    const i = argv.indexOf(name)
    return i >= 0 ? argv[i + 1] : undefined
  }
  const out = opt('--out')
  fetchConpty({ arch: opt('--arch'), out: out && path.resolve(out) }).catch((e) => {
    console.error(`could not fetch ConPTY: ${e.message}`)
    process.exit(1)
  })
}
