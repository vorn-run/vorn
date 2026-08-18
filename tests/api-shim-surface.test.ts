import { describe, it, expect } from 'vitest'
import fs from 'node:fs'
import path from 'node:path'

/**
 * The web shim must implement every method the preload does.
 *
 * The renderer is shared: the same components run in Electron against `window.api`
 * from the preload, and in the browser against the shim. A method present in one and
 * missing from the other is not a degraded feature — `App` subscribes to several at
 * mount, so one missing name throws during render and the entire web app fails to
 * appear, showing its loading screen forever with no error on screen.
 *
 * That is exactly what happened: the shim drifted 63 methods behind while nobody
 * rebuilt the web bundle, so the break stayed invisible for weeks. Comparing the two
 * source files is crude, but it is the only check that runs without building either
 * bundle, and it fails on the commit that introduces the drift rather than whenever
 * someone next happens to open the web app.
 */

const ROOT = path.resolve(__dirname, '..')

/** Top-level keys of the object literal each file exports, by indentation. */
function methodNames(file: string, indent: number): Set<string> {
  const source = fs.readFileSync(path.join(ROOT, file), 'utf8')
  const pattern = new RegExp(`^ {${indent}}([a-zA-Z_][\\w]*)\\s*:`, 'gm')
  const found = new Set<string>()
  for (const m of source.matchAll(pattern)) found.add(m[1])
  return found
}

describe('web api shim surface', () => {
  const preload = methodNames('src/preload/index.ts', 2)
  const shim = methodNames('packages/web/src/api-shim.ts', 4)

  it('reads both files, so a rename cannot make this test vacuously pass', () => {
    expect(preload.size).toBeGreaterThan(100)
    expect(shim.size).toBeGreaterThan(100)
  })

  it('implements every preload method', () => {
    const missing = [...preload].filter((name) => !shim.has(name) && !name.startsWith('_')).sort()

    expect(missing).toEqual([])
  })

  it('has no undeclared method the preload lacks', () => {
    // The other direction matters too: a shim-only method is one the Electron build
    // would throw on, which is the same failure with the platforms swapped. The
    // exception is a method the renderer feature-detects before calling, which is
    // allowed but has to be named here so it stays a decision rather than drift.
    const WEB_ONLY = new Set([
      // App.tsx:180 guards with `isWeb && 'listActiveSessions' in window.api`. The
      // web client reconnects to a server that outlived the page, so it has to ask
      // what is already running; Electron starts the server itself and never does.
      'listActiveSessions'
    ])
    const extra = [...shim]
      .filter((name) => !preload.has(name) && !name.startsWith('_') && !WEB_ONLY.has(name))
      .sort()

    expect(extra).toEqual([])
  })
})
