import { describe, expect, it } from 'vitest'
import {
  buildLoopVars,
  resolveTemplateValue,
  resolveTemplateVars
} from '../packages/shared/src/template-vars'
import type { WorkflowExecutionContext, WorkflowNode } from '../packages/shared/src/types'

const findings = [
  { path: 'a.ts', line: 3 },
  { path: 'b.ts', line: 9 }
]
const outputs = { review: { findings, output: 'done', status: 'success' } }

describe('resolveTemplateValue', () => {
  it('gives the list itself when the template is one reference to it', () => {
    expect(resolveTemplateValue('{{steps.review.findings}}', undefined, outputs)).toBe(findings)
    expect(resolveTemplateValue('  {{ steps.review.findings }} ', undefined, outputs)).toBe(
      findings
    )
  })

  it('never cuts a long list down to the text limit', () => {
    const long = Array.from({ length: 5000 }, (_, i) => ({ id: i, body: 'x'.repeat(20) }))
    const value = resolveTemplateValue('{{steps.big.items}}', undefined, { big: { items: long } })
    expect(value).toBe(long)
    // As text it is cut, which is why a loop must not read it as text.
    expect(
      resolveTemplateVars('{{steps.big.items}}', undefined, { big: { items: long } }).length
    ).toBe(50_000)
  })

  it('is text when the template holds more than the reference', () => {
    expect(resolveTemplateValue('found: {{steps.review.output}}', undefined, outputs)).toBe(
      'found: done'
    )
  })

  it('leaves a reference to nothing this run knows as it was written', () => {
    expect(resolveTemplateValue('{{nowhere.at.all}}', undefined, outputs)).toBe(
      '{{nowhere.at.all}}'
    )
  })
})

describe('the loop namespace', () => {
  const inside: WorkflowExecutionContext = {
    loop: { item: { path: 'a.ts', line: 3, tags: ['x'] }, index: 0, number: 1, count: 2 }
  }

  it('reads the item a pass is on, and where the pass is', () => {
    expect(resolveTemplateVars('{{loop.item.path}}:{{loop.item.line}}', inside)).toBe('a.ts:3')
    expect(
      resolveTemplateVars('{{loop.number}} of {{loop.count}} (index {{loop.index}})', inside)
    ).toBe('1 of 2 (index 0)')
    expect(resolveTemplateValue('{{loop.item.tags}}', inside)).toEqual(['x'])
    expect(resolveTemplateVars('{{loop.item}}', inside)).toBe(
      '{"path":"a.ts","line":3,"tags":["x"]}'
    )
  })

  it('stays as written outside a loop, so a misplaced reference shows', () => {
    expect(resolveTemplateVars('{{loop.item.path}}', {})).toBe('{{loop.item.path}}')
    expect(resolveTemplateVars('{{loop.whatever}}', inside)).toBe('{{loop.whatever}}')
  })
})

describe('buildLoopVars', () => {
  const node = (id: string, type: string, config: Record<string, unknown> = {}) =>
    ({ id, type, label: id, position: { x: 0, y: 0 }, config }) as unknown as WorkflowNode
  const nodes = [
    node('loop', 'loop', {
      nodeType: 'loop',
      mode: 'forEach',
      items: '{{steps.gate.items}}',
      bodyNodeIds: ['each']
    }),
    node('each', 'script'),
    node('after', 'script')
  ]

  it('offers {{loop.*}} to a step inside the loop and to the loop itself', () => {
    expect(buildLoopVars(nodes, 'each').map((v) => v.key)).toEqual([
      '{{loop.item}}',
      '{{loop.number}}',
      '{{loop.index}}',
      '{{loop.count}}'
    ])
    expect(buildLoopVars(nodes, 'loop')).toHaveLength(4)
  })

  it('offers nothing outside a loop, and no item to a repeating one', () => {
    expect(buildLoopVars(nodes, 'after')).toEqual([])
    const repeating = [
      node('loop', 'loop', { nodeType: 'loop', bodyNodeIds: ['each'] }),
      node('each', 'script')
    ]
    expect(buildLoopVars(repeating, 'each').map((v) => v.key)).not.toContain('{{loop.item}}')
  })
})
