// @vitest-environment jsdom
import { beforeEach, describe, expect, it } from 'vitest'
import { readRecentActions, recordRecentAction } from '../src/renderer/lib/step-library-memory'

beforeEach(() => localStorage.clear())

describe('the actions picked lately', () => {
  it('are the last five, newest first, each once', () => {
    for (let n = 1; n <= 6; n++) recordRecentAction({ connectorId: 'substack', action: `a${n}` })
    recordRecentAction({ connectorId: 'substack', action: 'a4', connectionId: 'novum' })
    expect(readRecentActions().map((a) => a.action)).toEqual(['a4', 'a6', 'a5', 'a3', 'a2'])
    expect(readRecentActions()[0].connectionId).toBe('novum')
  })

  it('are nothing when what was kept cannot be read', () => {
    localStorage.setItem('vorn:recentSteps', 'not json')
    expect(readRecentActions()).toEqual([])
    localStorage.setItem('vorn:recentSteps', JSON.stringify([{ kind: 'type', type: 'agent' }]))
    expect(readRecentActions()).toEqual([])
  })
})
