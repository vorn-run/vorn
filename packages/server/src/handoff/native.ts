import * as pty from 'node-pty'
import log from '../logger'

/**
 * Resizing a pty is a TIOCSWINSZ ioctl, which Node has no binding for.
 *
 * node-pty publishes its own binding as `native`, which is the only form that
 * survives bundling: `external` and the packaged app's module patch both match
 * the bare specifier, and a subpath is bundled instead -- taking node-pty's
 * loader with it, whose relative `require` then resolves against dist/ and finds
 * nothing. Absent from the typings, hence the cast and nothing more.
 */
const binding =
  (
    pty as unknown as {
      native?: {
        resize(
          fd: number,
          cols: number,
          rows: number,
          pixelWidth: number,
          pixelHeight: number
        ): void
      } | null
    }
  ).native ?? null

/** Answers rather than throws: a pane at the wrong width must not end a server holding every terminal. */
export function resizeFd(fd: number, cols: number, rows: number): boolean {
  if (!binding) return false
  try {
    binding.resize(fd, cols, rows, 0, 0)
    return true
  } catch (err) {
    log.warn({ err, fd, cols, rows }, '[handoff] could not resize an adopted pty')
    return false
  }
}
