/**
 * How a run asks to be let through.
 *
 * The run pane and the inline trace both render this pair, and they disagreed:
 * approve was blue in one and green in the other, so the same decision looked
 * like two different decisions depending on where you happened to be reading.
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
