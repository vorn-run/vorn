import { describe, expect, it } from 'vitest'
import type { WorkflowExecutionContext } from '../src/shared/types'
import { getAvailableContextVars, resolveTemplateVars } from '../src/renderer/lib/template-vars'

describe('webhook trigger variables', () => {
  it('offers only the request body and headers for a webhook trigger', () => {
    const vars = getAvailableContextVars({ triggerType: 'webhook', isContextualTrigger: false })
    expect(vars.map((v) => v.key)).toEqual(['{{trigger.body}}', '{{trigger.headers}}'])
  })

  it('keeps webhook variables out of task status triggers', () => {
    const vars = getAvailableContextVars({
      triggerType: 'taskStatusChanged',
      isContextualTrigger: false
    })
    const keys = vars.map((v) => v.key)
    expect(keys).toContain('{{trigger.fromStatus}}')
    expect(keys).not.toContain('{{trigger.body}}')
  })

  it('resolves nested request body and header paths', () => {
    const context: WorkflowExecutionContext = {
      trigger: {
        type: 'webhook',
        body: { order: { id: 42 } },
        headers: { 'x-event': 'deploy' },
        method: 'POST'
      }
    }
    expect(resolveTemplateVars('id={{trigger.body.order.id}}', context, {})).toBe('id=42')
    expect(resolveTemplateVars('ev={{trigger.headers.x-event}}', context, {})).toBe('ev=deploy')
  })
})
