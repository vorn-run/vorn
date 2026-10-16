import {
  endAll,
  leftAfter,
  leftoversMatching,
  type Process as Daemon
} from '../../scripts/lib/leftover-processes.mjs'

/**
 * The test daemons that end with process `pid`: those whose environment has
 * `VORN_EXIT_WITH` naming it, which is how a test file's worker tells its own
 * from any other run's on the machine. Needs the environment of other
 * processes, so it finds none on Windows.
 */
export function tiedTo(pid: number): Daemon[] {
  const mark = new RegExp(`VORN_EXIT_WITH=${pid}(\\s|$)`)
  return leftoversMatching((text) => mark.test(text))
}

/** The test daemons whose environment names this run's temporary root. */
export function inSandbox(sandboxRoot: string): Daemon[] {
  return leftoversMatching((text) => text.includes(`VORN_TEST_SANDBOX=${sandboxRoot}`))
}

/**
 * Waits up to `seconds` for `find`'s daemons to end on their own, then ends
 * the rest and says which they were. Answers an empty text when none was left.
 */
export async function leftOver(find: () => Daemon[], seconds: number): Promise<string> {
  const left = await leftAfter(find, seconds)
  if (left.length === 0) return ''
  await endAll(left)
  return `${left.length} test daemons outlived the tests:\n${left
    .map((d) => `  ${d.pid} ${d.label}`)
    .join('\n')}`
}
