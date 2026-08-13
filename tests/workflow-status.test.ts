import { describe, it, expect } from 'vitest'
import {
  WORKFLOW_STATUS_DOT,
  WORKFLOW_STATUS_DOT_PULSE,
  WORKFLOW_STATUS_TEXT,
  WORKFLOW_OUTCOME_TEXT,
  type WorkflowStatusKey
} from '../src/renderer/lib/workflow-status'

const EVERY_STATUS: WorkflowStatusKey[] = [
  'waiting',
  'error',
  'running',
  'success',
  'pending',
  'skipped',
  'cancelled'
]

describe('workflow status colours', () => {
  it('spends the accent only on the state that is blocked on the person', () => {
    // The whole rule, in one assertion. If a second status ever takes bronzo it
    // stops meaning "this needs you" and starts meaning "this is a workflow".
    const accented = EVERY_STATUS.filter((s) => WORKFLOW_STATUS_DOT[s].includes('bronzo'))
    expect(accented).toEqual(['waiting'])
  })

  it('reserves danger for the one outcome that actually broke', () => {
    // Cancelled is a decision, not a failure — it must not read as one.
    const dangerous = EVERY_STATUS.filter((s) => WORKFLOW_STATUS_DOT[s].includes('danger'))
    expect(dangerous).toEqual(['error'])
    expect(WORKFLOW_STATUS_DOT.cancelled).not.toContain('danger')
  })

  it('animates only what is still moving', () => {
    // A settled run that keeps pulsing reads as still going.
    const moving = EVERY_STATUS.filter((s) =>
      WORKFLOW_STATUS_DOT_PULSE[s].includes('animate-pulse')
    )
    expect(moving.sort()).toEqual(['running', 'waiting'])
    for (const s of EVERY_STATUS) {
      expect(WORKFLOW_STATUS_DOT[s]).not.toContain('animate-pulse')
    }
  })

  it('covers every status in all three maps', () => {
    // The old map left `waiting` and `cancelled` untested, which is how a dot
    // and its matching word drifted apart unnoticed.
    for (const s of EVERY_STATUS) {
      expect(WORKFLOW_STATUS_DOT[s]).toBeTruthy()
      expect(WORKFLOW_STATUS_DOT_PULSE[s]).toBeTruthy()
      expect(WORKFLOW_STATUS_TEXT[s]).toBeTruthy()
    }
  })

  it('says the same thing as a dot and as a word', () => {
    // These were separate maps that disagreed: running was a yellow dot but a
    // blue word, success a green dot but a grey outcome.
    for (const s of EVERY_STATUS) {
      expect(WORKFLOW_STATUS_TEXT[s]).toBe(WORKFLOW_STATUS_DOT[s].replace('bg-', 'text-'))
    }
  })

  it('gives a run outcome the same tone as the status behind it', () => {
    expect(WORKFLOW_OUTCOME_TEXT.waiting).toBe(WORKFLOW_STATUS_TEXT.waiting)
    expect(WORKFLOW_OUTCOME_TEXT.error).toBe(WORKFLOW_STATUS_TEXT.error)
    expect(WORKFLOW_OUTCOME_TEXT.running).toBe(WORKFLOW_STATUS_TEXT.running)
    expect(WORKFLOW_OUTCOME_TEXT.success).toBe(WORKFLOW_STATUS_TEXT.success)
  })
})
