import { describe, it, expect } from 'vitest'
import {
  TONE_DOT,
  TONE_DOT_MOVING,
  TONE_TEXT,
  type StatusTone
} from '../src/renderer/lib/status-tone'
import { STATUS_TONE } from '../src/renderer/lib/status-colors'
import { WORKFLOW_STATUS_TONE } from '../src/renderer/lib/workflow-status'

const EVERY_TONE: StatusTone[] = ['blocked', 'broken', 'live', 'settled', 'idle']

describe('the shared status tones', () => {
  it('spends the accent on exactly one meaning', () => {
    // The whole reason bronzo exists. A second tone taking it turns "this needs
    // you" into "this is a status", which is what it meant before phase 1.
    const accented = EVERY_TONE.filter((t) => TONE_DOT[t].includes('bronzo'))
    expect(accented).toEqual(['blocked'])
    expect(EVERY_TONE.filter((t) => TONE_TEXT[t].includes('bronzo'))).toEqual(['blocked'])
  })

  it('reserves danger for the one tone that means something broke', () => {
    expect(EVERY_TONE.filter((t) => TONE_DOT[t].includes('danger'))).toEqual(['broken'])
  })

  it('moves only what is actually working', () => {
    // A blocked state wants attention but is not doing anything; the accent is
    // what calls you to it. Motion stays a report of work in progress.
    const moving = EVERY_TONE.filter((t) => TONE_DOT_MOVING[t].includes('animate-pulse'))
    expect(moving).toEqual(['live'])
    for (const t of EVERY_TONE) {
      expect(TONE_DOT[t]).not.toContain('animate-pulse')
    }
  })

  it('keeps every tone legible as a word', () => {
    // A dot is a solid disc and a word is a thin stroke. Ghost reads at 1.5px
    // and vanishes as a label — phase 2 shipped a stopped run's verdict at
    // roughly 1.7:1 by treating the two maps as interchangeable.
    for (const t of EVERY_TONE) {
      expect(TONE_TEXT[t]).not.toContain('ghost')
    }
    expect(TONE_DOT.idle).toContain('ghost')
  })

  it('says the same thing as a dot and as a word', () => {
    // Same hue family either way; only the step on the ramp may differ.
    const family = (c: string): string =>
      c.replace(/^(bg|text)-/, '').replace(/-(faint|ghost)$/, '')
    for (const t of EVERY_TONE) {
      expect(family(TONE_TEXT[t])).toBe(family(TONE_DOT[t]))
    }
  })
})

describe('what each surface calls blocked', () => {
  it('agrees across sessions and workflows', () => {
    // Three surfaces, one accent. These used to be independent tables, so a
    // waiting session and a waiting gate could — and did — drift apart.
    expect(STATUS_TONE.waiting).toBe('blocked')
    expect(WORKFLOW_STATUS_TONE.waiting).toBe('blocked')
  })

  it('gives each surface exactly one blocked state', () => {
    const sessions = Object.values(STATUS_TONE).filter((t) => t === 'blocked')
    const workflows = Object.values(WORKFLOW_STATUS_TONE).filter((t) => t === 'blocked')
    expect(sessions).toHaveLength(1)
    expect(workflows).toHaveLength(1)
  })

  it('keeps a stopped run out of broken', () => {
    // Stopping a run is a decision, not a failure.
    expect(WORKFLOW_STATUS_TONE.cancelled).not.toBe('broken')
  })
})
