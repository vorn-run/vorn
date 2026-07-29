import type { ShellSetup, ShellOptions } from './types'

/**
 * cmd.exe: the PROMPT environment variable, and nothing else.
 *
 * cmd has no initialisation file we can inject and no hooks, but it does expand
 * `$e` in PROMPT to ESC, which is enough to bracket each prompt. This is the
 * recipe Microsoft documents for Windows Terminal.
 *
 * It is the weakest of the five by a wide margin, and unavoidably so:
 *
 *  - No pre-execution hook, so no C marker. The renderer treats the prompt as
 *    the block's start instead.
 *  - No way to read the previous command's status from a prompt, so D carries
 *    no exit code and a failed command looks like a successful one.
 *  - No way to report the command text, so a block is titled by the line the
 *    user typed, which is captured as output.
 *
 * A block still gets delimited, which is the part that matters for layout.
 */

/**
 * `$e` is ESC, `$P` the current path, `$G` a literal `>`. `$e\` is the string
 * terminator. D comes first, closing the command that has just finished, before
 * A opens the next prompt.
 */
const MARKERS_BEFORE = '$e]133;D$e\\$e]5522;cwd;$P$e\\$e]133;A$e\\'
const MARKERS_AFTER = '$e]133;B$e\\'

export function cmdSetup(opts: ShellOptions, existingPrompt: string): ShellSetup {
  // Wrap whatever the user already had rather than replacing it; `$P$G` is
  // cmd's own default when PROMPT is unset.
  const visible = opts.minimalPrompt ? '' : existingPrompt || '$P$G'
  return {
    env: { PROMPT: `${MARKERS_BEFORE}${visible}${MARKERS_AFTER}` },
    args: null
  }
}
