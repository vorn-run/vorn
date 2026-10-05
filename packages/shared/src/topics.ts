/**
 * What a phone asks the server for.
 *
 * Every namespace the web client registers a handler for, except terminal
 * bytes: those are asked for one terminal at a time, as cards come on screen.
 * `terminal:exit`, `terminal:bell` and `terminal:notify` stay by name because
 * the ended strip and the notifications need them for cards that are not on
 * screen.
 * `terminal:resync` is only ever sent about a terminal whose bytes a client was
 * receiving, so naming it costs nothing; `terminal:resized` likewise, which
 * vornd sends only to a client attached to the terminal.
 */
export const PHONE_BASE_TOPICS: readonly string[] = [
  'artifact:*',
  'config:*',
  'connector:*',
  'extension:*',
  'headless:*',
  'pairing:*',
  'scheduler:*',
  'script:*',
  'session:*',
  'workflow:*',
  'worktree:*',
  'terminal:exit',
  'terminal:bell',
  'terminal:notify',
  'terminal:resync',
  'terminal:resized'
]

/** The instance form the server's filter understands. */
export function terminalTopic(id: string): string {
  return `terminal:data#${id}`
}

export function topicsQuery(topics: readonly string[]): string {
  return `topics=${encodeURIComponent(topics.join(','))}`
}
