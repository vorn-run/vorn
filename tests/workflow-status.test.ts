import { describe, it, expect } from 'vitest'
import {
  WORKFLOW_STATUS_DOT,
  WORKFLOW_STATUS_DOT_PULSE,
  WORKFLOW_STATUS_TEXT,
  outcomeToneClass,
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

  it('animates only the state that is actually doing something', () => {
    // A waiting gate wants attention, but it is not working — bronzo is what
    // calls you to it. Motion stays a report of work in progress, the rule the
    // sessions surface already follows for a waiting session; the two
    // disagreeing meant one gate pulsed in the run list and sat still in the
    // dock at the same moment.
    const moving = EVERY_STATUS.filter((s) =>
      WORKFLOW_STATUS_DOT_PULSE[s].includes('animate-pulse')
    )
    expect(moving).toEqual(['running'])
    for (const s of EVERY_STATUS) {
      expect(WORKFLOW_STATUS_DOT[s]).not.toContain('animate-pulse')
    }
  })

  it('says the same thing as a dot and as a word', () => {
    // These were separate maps that disagreed: running was a yellow dot but a
    // blue word, success a green dot but a grey outcome.
    //
    // Sameness is the tone, not the exact value. A dot is a solid disc and a
    // word is a thin stroke, so the settled statuses sit a step brighter as
    // text — asserting strict equality here is what let a stopped run's verdict
    // ship at roughly 1.7:1.
    const tone = (c: string): string => c.replace(/^(bg|text)-/, '').replace(/-(faint|ghost)$/, '')
    for (const s of EVERY_STATUS) {
      expect(tone(WORKFLOW_STATUS_TEXT[s])).toBe(tone(WORKFLOW_STATUS_DOT[s]))
    }
  })

  it('keeps a status legible as a word even where the dot may recede', () => {
    // ink-ghost reads fine as a 1.5px disc and not at all as a label.
    for (const s of EVERY_STATUS) {
      expect(WORKFLOW_STATUS_TEXT[s]).not.toContain('ghost')
    }
  })

  it('gives a run outcome the same tone as the status behind it', () => {
    expect(outcomeToneClass('waiting')).toBe(WORKFLOW_STATUS_TEXT.waiting)
    expect(outcomeToneClass('error')).toBe(WORKFLOW_STATUS_TEXT.error)
    expect(outcomeToneClass('running')).toBe(WORKFLOW_STATUS_TEXT.running)
    expect(outcomeToneClass('success')).toBe(WORKFLOW_STATUS_TEXT.success)
    // A stopped run has no state of its own; settled is what it is.
    expect(outcomeToneClass('neutral')).toBe(WORKFLOW_STATUS_TEXT.success)
  })
})
