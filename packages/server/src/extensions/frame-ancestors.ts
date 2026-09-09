/** Origins the app draws its windows from, which only the launcher knows. */
const APP_ORIGIN = /^[a-z][a-z0-9+.-]*:(\/\/[^\s;,']+)?$/i

export function appFrameAncestors(declared: string | undefined): string[] {
  return (
    (declared ?? '')
      .split(',')
      .map((origin) => origin.trim())
      // Written straight into a header, which a space or a semicolon would rewrite.
      .filter((origin) => APP_ORIGIN.test(origin))
  )
}
