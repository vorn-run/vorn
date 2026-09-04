import { describe, it, expect } from 'vitest'
import { mkdtempSync, writeFileSync } from 'fs'
import { join } from 'path'
import { tmpdir } from 'os'
import {
  parseTemplates,
  parseMcpServers,
  catalogTemplates,
  catalogMcpServers,
  catalogSnapshot,
  refreshCatalog,
  resetCatalogCache
} from '../packages/server/src/connectors/catalog'
import { TEMPLATE_SEED } from '../packages/server/src/connectors/template-seed'
import { PORTABLE_FORMAT_VERSION } from '../packages/shared/src/workflow-portability'

/** Nothing in these tests may touch the network or this machine's real cache. */
function emptyCache(): string {
  return join(mkdtempSync(join(tmpdir(), 'vorn-templates-')), 'catalog.json')
}

function offline() {
  return {
    fetchImpl: (async () => {
      throw new Error('offline')
    }) as unknown as typeof fetch
  }
}

function published(id: string, extra: Record<string, unknown> = {}) {
  return {
    id,
    name: `Template ${id}`,
    description: 'Does a thing',
    steps: ['Webhook', 'Script'],
    portable: {
      version: PORTABLE_FORMAT_VERSION,
      slug: id,
      name: `Template ${id}`,
      nodes: [{ id: 'trigger', type: 'trigger', label: 'Webhook', config: {}, position: {} }],
      edges: []
    },
    ...extra
  }
}

const CONNECTORS = [{ id: 'c', name: 'C', packageName: '@vornrun/connector-c' }]

describe('parseTemplates', () => {
  it('reads what a catalog publishes', () => {
    const templates = parseTemplates({ version: 1, connectors: [], templates: [published('a')] })
    expect(templates.map((t) => t.id)).toEqual(['a'])
    expect(templates[0].steps).toEqual(['Webhook', 'Script'])
  })

  it('treats a document with no templates as having none, not as broken', () => {
    // Every catalog published so far carries connectors only.
    expect(parseTemplates({ version: 1, connectors: CONNECTORS })).toEqual([])
    expect(parseTemplates(null)).toEqual([])
    expect(parseTemplates({ version: 1, connectors: [], templates: 'soon' })).toEqual([])
  })

  it('drops one unusable template rather than the whole list', () => {
    const templates = parseTemplates({
      templates: [published('good'), { id: 'nameless' }, published('also-good')]
    })
    expect(templates.map((t) => t.id)).toEqual(['good', 'also-good'])
  })

  it('refuses a template whose workflow this build cannot read', () => {
    const future = published('future')
    future.portable.version = 99
    expect(parseTemplates({ templates: [future] })).toEqual([])
  })

  it('refuses a template carrying no steps to run', () => {
    const empty = published('empty')
    empty.portable.nodes = []
    expect(parseTemplates({ templates: [empty] })).toEqual([])
  })

  it('refuses to carry requirements that are not a list', () => {
    // Everything downstream walks this; a string here took the editor down.
    const hostile = published('hostile')
    ;(hostile.portable as Record<string, unknown>).requires = 'soon'
    const [template] = parseTemplates({ templates: [hostile] })
    expect(template.portable.requires).toEqual([])
  })

  it('drops requirement entries nothing could act on', () => {
    const hostile = published('hostile')
    ;(hostile.portable as Record<string, unknown>).requires = [
      'not an object',
      null,
      { kind: 'connection', nodeId: 'ok', connectorId: 'github', name: 'eng' },
      { kind: 'connection', nodeId: 42, connectorId: 'github', name: 'eng' },
      { kind: 'connection', nodeId: 'no-connector', name: 'eng' },
      { kind: 'httpProfile', nodeId: 'profile', name: 'reporting' },
      { kind: 'httpProfile', nodeId: 'nameless' },
      { kind: 'invented', nodeId: 'x', connectorId: 'y', name: 'z' }
    ]
    const [template] = parseTemplates({ templates: [hostile] })
    expect(template.portable.requires).toEqual([
      { kind: 'connection', nodeId: 'ok', connectorId: 'github', name: 'eng' },
      { kind: 'httpProfile', nodeId: 'profile', name: 'reporting' }
    ])
  })

  it('leaves a template that names no requirements alone', () => {
    const [template] = parseTemplates({ templates: [published('plain')] })
    expect(template.portable.requires).toBeUndefined()
  })

  it('repairs a template that is merely missing its blurb', () => {
    const bare = published('bare')
    delete (bare as Record<string, unknown>).description
    delete (bare as Record<string, unknown>).steps
    const [template] = parseTemplates({ templates: [bare] })
    expect(template).toMatchObject({ id: 'bare', description: '', steps: [] })
  })
})

describe('the bundled seed', () => {
  it('offers somewhere to start before anything is fetched', () => {
    resetCatalogCache()
    const templates = catalogTemplates({ ...offline(), cachePath: emptyCache() })
    expect(templates).toEqual(TEMPLATE_SEED)
    expect(templates.length).toBeGreaterThan(0)
  })

  it('stays small, because publishing is how the list grows', () => {
    expect(TEMPLATE_SEED.length).toBeLessThanOrEqual(5)
  })

  it('needs nothing installed, so a first run can use every one of them', () => {
    const connectorBound = TEMPLATE_SEED.flatMap((template) =>
      template.portable.nodes.filter(
        (node) =>
          node.type === 'callConnectorAction' ||
          (node.config as { triggerType?: string }).triggerType === 'connectorPoll'
      )
    )
    expect(connectorBound).toEqual([])
  })

  it('publishes no webhook token, which would be the same secret everywhere', () => {
    const tokens = TEMPLATE_SEED.flatMap((template) =>
      template.portable.nodes
        .map((node) => node.config as { triggerType?: string; token?: string })
        .filter((config) => config.triggerType === 'webhook')
        .map((config) => config.token)
    )
    expect(tokens.length).toBeGreaterThan(0)
    expect(tokens.every((token) => token === '')).toBe(true)
  })

  it('names every edge endpoint it draws', () => {
    for (const template of TEMPLATE_SEED) {
      const ids = new Set(template.portable.nodes.map((node) => node.id))
      for (const edge of template.portable.edges) {
        expect(ids.has(edge.source)).toBe(true)
        expect(ids.has(edge.target)).toBe(true)
      }
    }
  })

  it('keeps every loop body downstream of its own loop', () => {
    for (const template of TEMPLATE_SEED) {
      const byId = new Map(template.portable.nodes.map((node) => [node.id, node]))
      for (const node of template.portable.nodes) {
        if (node.type !== 'loop') continue
        const body = (node.config as { bodyNodeIds?: string[] }).bodyNodeIds ?? []
        expect(body.length).toBeGreaterThan(0)
        for (const id of body) expect(byId.get(id)).toBeDefined()
        // The engine refuses a gate inside a body, and the run would stop there.
        expect(body.some((id) => byId.get(id)?.type === 'approval')).toBe(false)
        // The chain the loop drives: loop → body[0] → body[1] → … with one edge each.
        const chain = [node.id, ...body]
        for (let i = 0; i < chain.length - 1; i += 1) {
          const hop = template.portable.edges.find(
            (edge) => edge.source === chain[i] && edge.target === chain[i + 1]
          )
          expect(hop, `${chain[i]} → ${chain[i + 1]}`).toBeDefined()
        }
      }
    }
  })
})
describe('parseMcpServers', () => {
  const server = { id: 'playwright', name: 'Playwright', command: 'npx', args: ['-y', 'mcp'] }

  it('reads what a catalog publishes', () => {
    expect(parseMcpServers({ mcpServers: [server] })).toEqual([
      { id: 'playwright', name: 'Playwright', command: 'npx', args: ['-y', 'mcp'] }
    ])
  })

  it('treats a document with no servers as having none', () => {
    expect(parseMcpServers({ version: 1, connectors: CONNECTORS })).toEqual([])
    expect(parseMcpServers(null)).toEqual([])
  })

  it('refuses an entry with nothing to start', () => {
    expect(parseMcpServers({ mcpServers: [{ id: 'x', name: 'X' }] })).toEqual([])
    expect(parseMcpServers({ mcpServers: [{ ...server, command: '' }] })).toEqual([])
  })

  it('repairs a list where a string was published instead', () => {
    const [entry] = parseMcpServers({
      mcpServers: [{ ...server, args: 'not a list', keywords: ['browser', 7] }]
    })
    expect(entry).toMatchObject({ args: [], keywords: ['browser'] })
  })

  it('has no seed, because a server nobody published is not a suggestion', () => {
    resetCatalogCache()
    expect(catalogMcpServers({ ...offline(), cachePath: emptyCache() })).toEqual([])
  })
})

describe('templates ride the catalog', () => {
  it('adopts published templates and caches them beside the connectors', async () => {
    resetCatalogCache()
    const cachePath = emptyCache()
    const fetchImpl = (async () =>
      new Response(
        JSON.stringify({ version: 1, connectors: CONNECTORS, templates: [published('remote')] })
      )) as unknown as typeof fetch

    expect(await refreshCatalog({ fetchImpl, cachePath, now: 42 })).toBe(true)
    expect(catalogTemplates({ ...offline(), cachePath }).map((t) => t.id)).toEqual(['remote'])

    resetCatalogCache()
    expect(catalogTemplates({ ...offline(), cachePath, now: 42 }).map((t) => t.id)).toEqual([
      'remote'
    ])
  })

  it('falls back to the seed for a cache written before templates existed', () => {
    resetCatalogCache()
    const cachePath = emptyCache()
    writeFileSync(cachePath, JSON.stringify({ fetchedAt: 5000, connectors: CONNECTORS }))

    expect(catalogTemplates({ ...offline(), now: 5000, cachePath })).toEqual(TEMPLATE_SEED)
  })

  it('hands the UI templates in the same answer as the connectors', () => {
    resetCatalogCache()
    const snapshot = catalogSnapshot({ ...offline(), cachePath: emptyCache() })
    expect(snapshot.templates).toEqual(TEMPLATE_SEED)
    expect(Array.isArray(snapshot.items)).toBe(true)
    expect(snapshot.mcpServers).toEqual([])
  })

  it('carries published servers through the cache too', async () => {
    resetCatalogCache()
    const cachePath = emptyCache()
    const server = { id: 'playwright', name: 'Playwright', command: 'npx', args: [] }
    const fetchImpl = (async () =>
      new Response(
        JSON.stringify({ version: 1, connectors: CONNECTORS, mcpServers: [server] })
      )) as unknown as typeof fetch

    expect(await refreshCatalog({ fetchImpl, cachePath, now: 7 })).toBe(true)
    resetCatalogCache()
    expect(catalogMcpServers({ ...offline(), cachePath, now: 7 }).map((s) => s.id)).toEqual([
      'playwright'
    ])
  })
})
