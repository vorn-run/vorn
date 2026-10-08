import { describe, it, expect } from 'vitest'
import { TEMPLATE_SEED } from './helpers/template-seed'

describe('the bundled seed', () => {
  it('offers somewhere to start before anything is fetched', () => {
    expect(TEMPLATE_SEED.length).toBeGreaterThan(0)
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
