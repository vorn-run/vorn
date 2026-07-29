import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest'

vi.mock('../packages/server/src/logger', () => ({
  default: { info: vi.fn(), warn: vi.fn(), error: vi.fn() }
}))

import {
  claimWorkflowRun,
  releaseWorkflowRun,
  resetWorkflowRunClaims,
  DEFAULT_DEDUPE_WINDOW_MS
} from '../packages/server/src/workflow-run-claims'

beforeEach(() => {
  resetWorkflowRunClaims()
  vi.useFakeTimers()
})

afterEach(() => {
  vi.useRealTimers()
})

describe('workflow run claims', () => {
  it('grants the first claim and refuses an identical one in the same window', () => {
    const first = claimWorkflowRun({ workflowId: 'wf-1', params: 'manual' })
    const second = claimWorkflowRun({ workflowId: 'wf-1', params: 'manual' })

    expect(first.granted).toBe(true)
    expect(second.granted).toBe(false)
    // The loser learns which run won, so it can show that run instead.
    expect(second.runId).toBe(first.runId)
  })

  it('runs the same workflow in parallel when the trigger parameters differ', () => {
    // A connector poll fans out one run per new item: same workflow, different
    // item. These are different work and must not suppress each other.
    const itemA = claimWorkflowRun({ workflowId: 'wf-1', params: 'item:conn-1:issue-7' })
    const itemB = claimWorkflowRun({ workflowId: 'wf-1', params: 'item:conn-1:issue-8' })

    expect(itemA.granted).toBe(true)
    expect(itemB.granted).toBe(true)
    expect(itemA.runId).not.toBe(itemB.runId)
  })

  it('keeps claims for different workflows independent', () => {
    expect(claimWorkflowRun({ workflowId: 'wf-1', params: 'manual' }).granted).toBe(true)
    expect(claimWorkflowRun({ workflowId: 'wf-2', params: 'manual' }).granted).toBe(true)
  })

  it('lets the trigger through again once the window lapses', () => {
    const first = claimWorkflowRun({ workflowId: 'wf-1', params: 'manual' })
    vi.advanceTimersByTime(DEFAULT_DEDUPE_WINDOW_MS + 1)

    const later = claimWorkflowRun({ workflowId: 'wf-1', params: 'manual' })
    expect(later.granted).toBe(true)
    expect(later.runId).not.toBe(first.runId)
  })

  it('frees the trigger immediately when the holder releases it', () => {
    const first = claimWorkflowRun({ workflowId: 'wf-1', params: 'manual' })
    releaseWorkflowRun('wf-1', 'manual', first.runId)

    // No time has passed, so only the release can explain this being granted.
    expect(claimWorkflowRun({ workflowId: 'wf-1', params: 'manual' }).granted).toBe(true)
  })

  it('ignores a release from a run that no longer holds the claim', () => {
    const first = claimWorkflowRun({ workflowId: 'wf-1', params: 'manual' })
    releaseWorkflowRun('wf-1', 'manual', first.runId)
    const second = claimWorkflowRun({ workflowId: 'wf-1', params: 'manual' })

    // A late release from the superseded run must not hand away the claim its
    // successor now owns.
    releaseWorkflowRun('wf-1', 'manual', first.runId)

    const third = claimWorkflowRun({ workflowId: 'wf-1', params: 'manual' })
    expect(third.granted).toBe(false)
    expect(third.runId).toBe(second.runId)
  })

  it('treats a missing fingerprint as the manual trigger', () => {
    claimWorkflowRun({ workflowId: 'wf-1' })
    expect(claimWorkflowRun({ workflowId: 'wf-1', params: 'manual' }).granted).toBe(false)
  })

  it('does not let a fingerprint containing the key separator collide', () => {
    // Fingerprints carry upstream text; a naive `id + sep + params` key would
    // let these two land on the same entry.
    const a = claimWorkflowRun({ workflowId: 'wf', params: 'x:y' })
    const b = claimWorkflowRun({ workflowId: 'wf:x', params: 'y' })
    expect(a.granted).toBe(true)
    expect(b.granted).toBe(true)
  })

  it('honours a caller-supplied window', () => {
    claimWorkflowRun({ workflowId: 'wf-1', params: 'manual', windowMs: 1000 })
    vi.advanceTimersByTime(1500)
    expect(claimWorkflowRun({ workflowId: 'wf-1', params: 'manual', windowMs: 1000 }).granted).toBe(
      true
    )
  })
})
