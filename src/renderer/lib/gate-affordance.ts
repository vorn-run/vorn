/**
 * How a run asks to be let through.
 *
 * Three places render this decision — the run pane, the inline trace, and the
 * approval pill on a session card — and they disagreed: approve was blue in
 * one and green in another, so the same decision looked like two different
 * decisions depending on where you happened to be reading. The pill keeps its
 * own icon-button sizing (see `icon-button.ts`) because it sits in a 26px
 * strip, but it takes the same two tones.
 *
 * Approve carries the accent because a waiting gate is the definition of work
 * blocked on the person — it is the one affordance in a workflow that bronzo
 * was minted for. Reject stays neutral until you reach for it: rejecting is
 * destructive, but offering it in danger at rest reads as a warning about the
 * run rather than a description of the button.
 */
export const GATE_APPROVE =
  'rounded-md border border-bronzo/40 bg-bronzo/10 text-bronzo hover:bg-bronzo/20 transition-colors'

export const GATE_REJECT =
  'rounded-md border border-white/[0.06] text-ink-secondary hover:bg-danger/10 hover:text-danger hover:border-danger/30 transition-colors'
