/**
 * Origins the app that spawned this server draws its windows from.
 *
 * A desktop window is not served by this server — it is a `file:` page, or a dev
 * server's — so it cannot be inferred from a port the way the web client's origin
 * is. Only the launcher knows, and it says so in `VORN_APP_ORIGINS`; a server
 * started from the CLI has no such window and names none.
 */
const APP_ORIGIN = /^[a-z][a-z0-9+.-]*:(\/\/[^\s;,']+)?$/i

export function appFrameAncestors(declared: string | undefined): string[] {
  return (
    (declared ?? '')
      .split(',')
      .map((origin) => origin.trim())
      // Written straight into a header: one carrying a space or a semicolon would
      // either widen what may frame a page or make the header unserveable.
      .filter((origin) => APP_ORIGIN.test(origin))
  )
}
