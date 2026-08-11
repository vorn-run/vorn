import { describe, it, expect } from 'vitest'
import { mkdirSync, mkdtempSync, readFileSync, writeFileSync } from 'fs'
import { tmpdir } from 'os'
import { join } from 'path'
import { defineConnector } from '../packages/connector-sdk/src'
import {
  CONNECTOR_CATALOG,
  catalogItems,
  catalogLaunchSpec,
  parseCatalog,
  refreshCatalog,
  resetCatalogCache
} from '../packages/server/src/connectors/catalog'

const ENTRY = CONNECTOR_CATALOG[0]

/** A cache path in a fresh directory, so no test reads this machine's own. */
function emptyCache(): string {
  return join(mkdtempSync(join(tmpdir(), 'catalog-')), 'connector-catalog.json')
}

/** Nothing in these tests may touch the network. */
function offline() {
  return {
    fetchImpl: (async () => {
      throw new Error('no network in tests')
    }) as unknown as typeof fetch
  }
}

function published(id: string) {
  return {
    id,
    name: id,
    description: 'd',
    packageName: `@vornrun/connector-${id}`,
    capabilities: ['triggers'] as const
  }
}

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
    // The connectors repository lays a package out as `packages/<id>`, not
    // `packages/connector-<id>` the way this repo used to.
    const dist = join(root, 'packages', ENTRY.id, 'dist')
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
    // be started next to, so the root is set deliberately, never sniffed.
    expect(catalogLaunchSpec(ENTRY, undefined).command).toBe('npx')
  })
})

describe('parseCatalog', () => {
  const entry = {
    id: 'x',
    name: 'X',
    description: 'd',
    packageName: '@vornrun/connector-x',
    capabilities: ['triggers']
  }

  it('reads a published catalog', () => {
    expect(parseCatalog({ version: 1, connectors: [entry] })).toEqual([entry])
  })

  it('carries what a listing needs to answer "will this do what I need"', () => {
    const rich = {
      ...entry,
      version: '1.2.3',
      triggers: [{ type: 'thing', label: 'A thing happened' }],
      actions: [{ type: 'do', label: 'Do it' }],
      env: [{ name: 'X_TOKEN', required: true }]
    }
    expect(parseCatalog({ version: 1, connectors: [rich] })?.[0]).toEqual(rich)
  })

  it('refuses a document from a format it does not understand', () => {
    // Falling back to what we already had beats emptying the connector list.
    expect(parseCatalog({ version: 2, connectors: [entry] })).toBeUndefined()
    expect(parseCatalog({ connectors: [entry] })).toBeUndefined()
    expect(parseCatalog('<!DOCTYPE html>')).toBeUndefined()
    expect(parseCatalog(undefined)).toBeUndefined()
  })

  it('drops a single unusable entry rather than the whole list', () => {
    const parsed = parseCatalog({
      version: 1,
      connectors: [{ id: 'broken' }, entry, { ...entry, packageName: '' }]
    })
    expect(parsed).toEqual([entry])
  })

  it('treats a list of nothing usable as no answer at all', () => {
    expect(parseCatalog({ version: 1, connectors: [] })).toBeUndefined()
    expect(parseCatalog({ version: 1, connectors: [{ id: 'broken' }] })).toBeUndefined()
  })
})

describe('the bundled seed', () => {
  it('is only a first-run fallback, so it stays small enough not to drift', () => {
    // The published catalog carries triggers, actions and settings. Copying
    // those here would recreate exactly the staleness the fetch exists to fix.
    for (const entry of CONNECTOR_CATALOG) {
      expect(entry.triggers).toBeUndefined()
      expect(entry.actions).toBeUndefined()
    }
  })
})

describe('catalogItems', () => {
  it('gives every entry a launch spec, which is what Add needs', () => {
    for (const item of catalogItems()) {
      expect(item.launch.command).toMatch(/\S/)
      expect(item.launch.args.length).toBeGreaterThan(0)
    }
  })

  it('resolves once, so opening settings twice does not re-read the disk', () => {
    expect(catalogItems()).toBe(catalogItems())
  })

  it('serves the seed immediately rather than waiting on a fetch', () => {
    // The connector list must render offline, and on the very first run there
    // is nothing cached to render from.
    resetCatalogCache()
    const items = catalogItems({ ...offline(), cachePath: emptyCache() })
    expect(items.map((item) => item.id)).toEqual(CONNECTOR_CATALOG.map((e) => e.id))
  })

  it('prefers a cached catalog over the seed', () => {
    resetCatalogCache()
    const cachePath = emptyCache()
    writeFileSync(cachePath, JSON.stringify({ fetchedAt: 1000, connectors: [published('fresh')] }))
    const items = catalogItems({ ...offline(), now: 1000, cachePath })
    expect(items.map((item) => item.id)).toEqual(['fresh'])
  })
})

describe('refreshCatalog', () => {
  it('adopts the published catalog and caches it for next time', async () => {
    resetCatalogCache()
    const cachePath = emptyCache()
    const fetchImpl = (async () =>
      new Response(
        JSON.stringify({ version: 1, connectors: [published('remote')] })
      )) as unknown as typeof fetch

    expect(await refreshCatalog({ fetchImpl, cachePath, now: 42 })).toBe(true)
    expect(catalogItems({ ...offline(), cachePath }).map((i) => i.id)).toEqual(['remote'])
    expect(JSON.parse(readFileSync(cachePath, 'utf8'))).toMatchObject({ fetchedAt: 42 })
  })

  it('keeps what it had when the fetch fails', async () => {
    resetCatalogCache()
    const cachePath = emptyCache()
    const fetchImpl = (async () => {
      throw new Error('offline')
    }) as unknown as typeof fetch

    expect(await refreshCatalog({ fetchImpl, cachePath })).toBe(false)
    // Emptying the connector list because a proxy blocked one request would
    // make the app look broken.
    expect(catalogItems({ ...offline(), cachePath }).length).toBeGreaterThan(0)
  })

  it('refuses a page served where the catalog should be', async () => {
    resetCatalogCache()
    const fetchImpl = (async () => new Response('<!DOCTYPE html>')) as unknown as typeof fetch
    expect(await refreshCatalog({ fetchImpl, cachePath: emptyCache() })).toBe(false)
  })

  it('refuses anything that is not a 200', async () => {
    resetCatalogCache()
    const fetchImpl = (async () => new Response('nope', { status: 404 })) as unknown as typeof fetch
    expect(await refreshCatalog({ fetchImpl, cachePath: emptyCache() })).toBe(false)
  })
})
