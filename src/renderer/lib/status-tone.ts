/**
 * The five things a status can mean, and what each looks like.
 *
 * Sessions, workflows and tasks all report state, and all three had been writing
 * the same five answers out separately — which is how a workflow's `running`
 * ended up a different colour from a session's, and how one task status ended up
 * with six values across five maps. A status vocabulary now says which *tone* a
 * state carries; this file is the only place that decides what a tone looks like.
 *
 * `blocked` is the accent, and it is the whole reason the accent exists: a
 * waiting session, an open approval gate, a task handed back for review. If a
 * second meaning ever takes bronzo it stops saying "this needs you" and starts
 * saying "this is a status", so the rule is enforced by test rather than by
 * memory — see `status-tone.test.ts`.
 */
export type StatusTone = 'blocked' | 'broken' | 'live' | 'settled' | 'idle'

export const TONE_DOT: Record<StatusTone, string> = {
  blocked: 'bg-bronzo',
  broken: 'bg-danger',
  live: 'bg-ink',
  settled: 'bg-ink-faint',
  idle: 'bg-ink-ghost'
}

/**
 * Not the dots with the prefix swapped.
 *
 * A dot is a solid 1.5px disc and a word is a thin stroke, so the two need
 * different floors on the ramp: `idle` reads fine as a disc and disappears as a
 * label. Phase 2 shipped a stopped run's headline verdict at roughly 1.7:1 by
 * assuming the two maps were interchangeable, so text bottoms out at faint.
 *
 * Both are spelled out rather than derived because Tailwind scans source text
 * for candidate class names — a name assembled at runtime is one it never sees,
 * and the rule would simply not be generated.
 */
export const TONE_TEXT: Record<StatusTone, string> = {
  blocked: 'text-bronzo',
  broken: 'text-danger',
  live: 'text-ink',
  settled: 'text-ink-faint',
  idle: 'text-ink-faint'
}

/**
 * Motion is a report of work in progress, so only `live` moves. A blocked state
 * wants attention rather than announcing activity, and the accent is what calls
 * you to it — a pulsing gate beside a still one said the two were different
 * kinds of waiting when they were the same.
 */
export const TONE_DOT_MOVING: Record<StatusTone, string> = {
  ...TONE_DOT,
  live: `${TONE_DOT.live} animate-pulse`
}
