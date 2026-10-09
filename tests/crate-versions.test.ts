import { describe, it, expect } from 'vitest'
import { readFileSync, readdirSync, existsSync, statSync } from 'node:fs'
import path from 'node:path'

const root = path.resolve(__dirname, '..')
const core = path.join(root, 'packages/core')
const appVersion: string = JSON.parse(readFileSync(path.join(root, 'package.json'), 'utf8')).version

function rustFiles(dir: string): string[] {
  if (!existsSync(dir)) return []
  return readdirSync(dir).flatMap((name) => {
    const file = path.join(dir, name)
    if (statSync(file).isDirectory()) return rustFiles(file)
    return name.endsWith('.rs') ? [file] : []
  })
}

/** Every crate whose own code reports `CARGO_PKG_VERSION`, by manifest directory. */
function reportingCrates(): { dir: string; name: string; manifest: string }[] {
  const dirs = readdirSync(path.join(core, 'crates')).map((c) => path.join(core, 'crates', c))
  return dirs
    .filter((dir) =>
      rustFiles(path.join(dir, 'src')).some((f) =>
        readFileSync(f, 'utf8').includes('env!("CARGO_PKG_VERSION")')
      )
    )
    .map((dir) => {
      const manifest = readFileSync(path.join(dir, 'Cargo.toml'), 'utf8')
      const name = /^\[package\]\r?\nname = "([^"]+)"/m.exec(manifest)?.[1] ?? ''
      return { dir, name, manifest }
    })
}

function packageSection(manifest: string): string {
  const start = manifest.indexOf('[package]')
  const rest = manifest.slice(start + '[package]'.length)
  const end = rest.search(/^\[/m)
  return end === -1 ? rest : rest.slice(0, end)
}

describe('the versions the native binaries report', () => {
  const crates = reportingCrates()

  it('finds the binaries that report a version to the app', () => {
    expect(crates.map((c) => c.name)).toEqual(
      expect.arrayContaining(['vorn-cli', 'vorn-sessiond', 'vornd'])
    )
  })

  it('takes the workspace version, which is the app version', () => {
    const workspace = readFileSync(path.join(core, 'Cargo.toml'), 'utf8')
    expect(/\[workspace\.package\]\r?\nversion = "([^"]+)"/.exec(workspace)?.[1]).toBe(appVersion)
    const notInheriting = crates
      .filter((c) => !/^version\.workspace = true\r?$/m.test(packageSection(c.manifest)))
      .map((c) => c.name)
    expect(notInheriting).toEqual([])
  })

  it('locks every one of them at the app version', () => {
    const lock = readFileSync(path.join(core, 'Cargo.lock'), 'utf8')
    const locked = crates.map(({ name }) => ({
      name,
      version: new RegExp(`name = "${name}"\\r?\\nversion = "([^"]+)"`).exec(lock)?.[1]
    }))
    expect(locked).toEqual(crates.map(({ name }) => ({ name, version: appVersion })))
  })

  it('has the sync script find inheriting crates from their manifests, not a list', () => {
    const script = readFileSync(path.join(root, 'scripts/sync-versions.sh'), 'utf8')
    expect(script).toContain('readdirSync(base)')
    expect(script).not.toMatch(/for \(const name of \['vorn-core'/)
  })
})
