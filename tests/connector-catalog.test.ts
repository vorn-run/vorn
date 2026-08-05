import { describe, it, expect } from 'vitest'
import { mkdirSync, mkdtempSync, writeFileSync } from 'fs'
import { tmpdir } from 'os'
import { join } from 'path'
import { defineConnector } from '../packages/connector-sdk/src'
import {
  CONNECTOR_CATALOG,
  catalogItems,
  catalogLaunchSpec
} from '../packages/server/src/connectors/catalog'

const ENTRY = CONNECTOR_CATALOG[0]

describe('connector catalog', () => {
  it('names a package for every entry so nothing has to be typed to install it', () => {
    expect(CONNECTOR_CATALOG.length).toBeGreaterThan(0)
    for (const entry of CONNECTOR_CATALOG) {
      expect(entry.packageName).toMatch(/\S/)
      expect(entry.name).toMatch(/\S/)
      expect(entry.description).toMatch(/\S/)
      expect(entry.capabilities.length).toBeGreaterThan(0)
    }
  })

  it('keeps entry ids unique so a card cannot shadow another', () => {
    const ids = CONNECTOR_CATALOG.map((e) => e.id)
    expect(new Set(ids).size).toBe(ids.length)
  })

  it('ships icons the connector SDK itself accepts, so a card is never blank', () => {
    // Validated with the real validator rather than a copy of its rules: a
    // copy keeps passing after the rules tighten, while the icon it approved
    // renders as nothing.
    for (const entry of CONNECTOR_CATALOG) {
      if (!entry.icon) continue
      expect(() =>
        defineConnector({
          id: entry.id,
          name: entry.name,
          version: '0.0.0',
          icon: entry.icon,
          triggers: [],
          actions: [{ type: 'noop', label: 'No-op', run: () => undefined }]
        })
      ).not.toThrow()
    }
  })
})

describe('catalogLaunchSpec', () => {
  it('resolves the published package when not running from a checkout', () => {
    expect(catalogLaunchSpec(ENTRY, undefined)).toEqual({
      command: 'npx',
      args: ['-y', ENTRY.packageName]
    })
  })

  it('prefers the local build in a checkout, so an unpublished connector still runs', () => {
    const root = mkdtempSync(join(tmpdir(), 'catalog-'))
    const dist = join(root, 'packages', ENTRY.packageName.replace(/^@[^/]+\//, ''), 'dist')
    mkdirSync(dist, { recursive: true })
    writeFileSync(join(dist, 'index.js'), '')

    expect(catalogLaunchSpec(ENTRY, root)).toEqual({
      command: 'node',
      args: [join(dist, 'index.js')]
    })
  })

  it('falls back to the package when a checkout has not built the connector', () => {
    const root = mkdtempSync(join(tmpdir(), 'catalog-'))

    expect(catalogLaunchSpec(ENTRY, root)).toEqual({
      command: 'npx',
      args: ['-y', ENTRY.packageName]
    })
  })

  it('never treats a stray directory as a checkout unless the root was given', () => {
    // A released app must not run whatever `packages/` folder it happened to
    // be started next to, so the repo root is passed in, never sniffed.
    expect(catalogLaunchSpec(ENTRY, undefined).command).toBe('npx')
  })
})

describe('catalogItems', () => {
  it('gives every entry a launch spec, which is what Add needs', () => {
    for (const item of catalogItems()) {
      expect(item.launch.command).toMatch(/\S/)
      expect(item.launch.args.length).toBeGreaterThan(0)
    }
  })

  it('resolves once, since neither the catalog nor the checkout moves at run time', () => {
    expect(catalogItems()).toBe(catalogItems())
  })
})
