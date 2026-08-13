import type { AgentStatus } from '../../shared/types'
import { TONE_DOT, TONE_TEXT, type StatusTone } from './status-tone'

/**
 * What each session state means, in the shared vocabulary.
 *
 * Only `waiting` is blocked on the person, so only `waiting` takes the accent.
 * Running is the commonest state — painting it would put bronzo across most of
 * the screen most of the time — so it reads as live, which is also the one tone
 * allowed to move. `status-tone.ts` decides what those words look like.
 */
export const STATUS_TONE: Record<AgentStatus, StatusTone> = {
  running: 'live',
  waiting: 'blocked',
  idle: 'idle',
  error: 'broken'
}

export const STATUS_DOT: Record<AgentStatus, string> = {
  running: TONE_DOT[STATUS_TONE.running],
  waiting: TONE_DOT[STATUS_TONE.waiting],
  idle: TONE_DOT[STATUS_TONE.idle],
  error: TONE_DOT[STATUS_TONE.error]
}

export const STATUS_TEXT: Record<AgentStatus, string> = {
  running: TONE_TEXT[STATUS_TONE.running],
  waiting: TONE_TEXT[STATUS_TONE.waiting],
  idle: TONE_TEXT[STATUS_TONE.idle],
  error: TONE_TEXT[STATUS_TONE.error]
}

export const STATUS_LABEL: Record<AgentStatus, string> = {
  running: 'Running',
  waiting: 'Waiting',
  idle: 'Idle',
  error: 'Error'
}
