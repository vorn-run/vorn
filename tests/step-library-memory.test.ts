// @vitest-environment jsdom
import { beforeEach, describe, expect, it } from 'vitest'
import {
  actionKey,
  readOpenGroups,
  readRecentPicks,
  recordRecentPick,
  writeOpenGroups,
  type RecentPick
} from '../src/renderer/lib/step-library-memory'

beforeEach(() => localStorage.clear())

const comment = (n: number): RecentPick => ({
  kind: 'connectorAction',
  connectionId: 'substack',
  action: `comment${n}`,
  actionLabel: `Comment ${n}`
})

describe('the actions picked lately', () => {
  it('are the last five, newest first, each once', () => {
    for (let n = 1; n <= 6; n++) recordRecentPick(comment(n))
    recordRecentPick(comment(4))
    expect(readRecentPicks().map((p) => p.action)).toEqual([
      'comment4',
      'comment6',
      'comment5',
      'comment3',
      'comment2'
    ])
  })

  it('are listed under the same row as in their group', () => {
    expect(actionKey(comment(1))).toBe('action:substack:comment1')
    expect(
      actionKey({
        kind: 'catalogAction',
        connectorId: 'slack',
        action: 'post',
        actionLabel: 'Post'
      })
    ).toBe('catalog:slack:post')
  })

  it('are nothing when what was kept cannot be read', () => {
    localStorage.setItem('vorn:recentSteps', 'not json')
    expect(readRecentPicks()).toEqual([])
    localStorage.setItem('vorn:recentSteps', JSON.stringify([{ kind: 'type', type: 'agent' }]))
    expect(readRecentPicks()).toEqual([])
  })
})

describe('the groups left open', () => {
  it('open the way they were left', () => {
    expect(readOpenGroups().size).toBe(0)
    writeOpenGroups(new Set(['group:c1', 'catalog-group:slack']))
    expect([...readOpenGroups()]).toEqual(['group:c1', 'catalog-group:slack'])
  })
})
