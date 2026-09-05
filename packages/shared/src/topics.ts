/**
 * What a phone asks the server for.
 *
 * Every namespace the web client registers a handler for, except terminal
 * bytes: those are asked for one terminal at a time, as cards come on screen.
 * `terminal:exit` and `terminal:bell` stay by name because the ended strip and
 * the notification need them for cards that are not on screen.
 */
export const PHONE_BASE_TOPICS: readonly string[] = [
  'config:*',
  'connector:*',
  'headless:*',
  'pairing:*',
  'scheduler:*',
  'script:*',
  'session:*',
  'workflow:*',
  'worktree:*',
  'terminal:exit',
  'terminal:bell'
]

/** The instance form the server's filter understands. */
export function terminalTopic(id: string): string {
  return `terminal:data#${id}`
}

export function topicsQuery(topics: readonly string[]): string {
  return `topics=${encodeURIComponent(topics.join(','))}`
}
