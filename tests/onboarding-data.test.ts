import { describe, it, expect } from 'vitest'
import { ONBOARDING_STEPS } from '../packages/web/src/renderer/lib/onboarding-data'

describe('ONBOARDING_STEPS', () => {
  it('has 7 steps', () => {
    expect(ONBOARDING_STEPS).toHaveLength(7)
  })

  it('each step has required fields', () => {
    for (const step of ONBOARDING_STEPS) {
      expect(step.id).toBeTruthy()
      expect(step.title).toBeTruthy()
      expect(step.subtitle).toBeTruthy()
      expect(step.icon).toBeTruthy()
    }
  })

  it('has unique step IDs', () => {
    const ids = ONBOARDING_STEPS.map((s) => s.id)
    expect(new Set(ids).size).toBe(ids.length)
  })

  it('steps are in expected order', () => {
    const ids = ONBOARDING_STEPS.map((s) => s.id)
    expect(ids).toEqual([
      'agents',
      'projects',
      'sessions',
      'workspace',
      'workflows',
      'shortcuts',
      'ready'
    ])
  })
})
