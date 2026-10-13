import { describe, it, expect } from 'vitest'
import { createCallConnectorActionNode } from '../packages/web/src/renderer/lib/workflow-helpers'
import type { CallConnectorActionConfig } from '../packages/web/src/shared/types'

describe('createCallConnectorActionNode', () => {
  it('produces a node with type callConnectorAction and empty defaults', () => {
    const node = createCallConnectorActionNode()
    expect(node.type).toBe('callConnectorAction')
    const cfg = node.config as CallConnectorActionConfig
    expect(cfg.nodeType).toBe('callConnectorAction')
    expect(cfg.connectionId).toBe('')
    expect(cfg.action).toBe('')
    expect(cfg.args).toEqual({})
  })

  it('gives each new node a unique id and a stable slug', () => {
    const a = createCallConnectorActionNode()
    const b = createCallConnectorActionNode()
    expect(a.id).not.toBe(b.id)
    expect(a.slug).toBeTruthy()
    expect(a.slug).toBe(b.slug) // same label → same slug; uniqueness is handled by the editor
  })

  it('accepts a partial config override', () => {
    const node = createCallConnectorActionNode({
      connectionId: 'conn-1',
      action: 'createIssue',
      args: { title: 'x' }
    })
    const cfg = node.config as CallConnectorActionConfig
    expect(cfg.connectionId).toBe('conn-1')
    expect(cfg.action).toBe('createIssue')
    expect(cfg.args).toEqual({ title: 'x' })
  })

  it('positions the node at the default origin', () => {
    const node = createCallConnectorActionNode()
    expect(node.position).toEqual({ x: 0, y: 0 })
  })
})
