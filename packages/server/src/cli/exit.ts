/**
 * What the process leaves behind, so a script can branch on it.
 *
 * 3 is missing deliberately: the server already exits with it for
 * `EXIT_ENDPOINT_TAKEN` (`packages/shared/src/protocol.ts`), and `vorn server
 * serve` passes that code through.
 */
export const EXIT_OK = 0
export const EXIT_FAILURE = 1
export const EXIT_USAGE = 2
export const EXIT_UNREACHABLE = 4
