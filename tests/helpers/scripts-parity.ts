/**
 * What may differ between a project script the server runs, while vornd is
 * not connected, and the same script vornd runs, and nothing else.
 *
 * A run is what `script:execute` answered, the output the clients were told
 * (`SCRIPT_DATA`) and the exits they were told (`SCRIPT_EXIT`).
 * {@link normalizeScriptRun} applies each accepted difference below, and the
 * two must then be equal.
 *
 * - {@link SCRIPT_STREAMS}: the session holder reads a script's stdout and
 *   stderr as one stream, so vornd's answer has both in `output`, and an
 *   `error` that is all it printed; the server's has stdout in `output` and
 *   stderr in `error`. Each answer is compared as the lines it printed, in
 *   order of their text, and whether it failed.
 * - {@link SCRIPT_CHUNKS}: the output reaches the clients in chunks cut where
 *   the pipe happened to deliver them, so it is compared whole, as its lines in
 *   order of their text: the server reads the two streams on pipes of their
 *   own, which interleave as they arrive.
 */

export const SCRIPT_STREAMS = 'script-stdout-and-stderr-read-as-one'
export const SCRIPT_CHUNKS = 'script-output-chunk-boundaries'

/** What `script:execute` answers. */
export interface ScriptExecutionResult {
  success: boolean
  output: string
  error?: string
  exitCode?: number
}

/** One run of a script, as the server's caller and its clients saw it. */
export interface ScriptRun {
  result: ScriptExecutionResult
  told: string[]
  exits: number[]
}

/** Who ran the script. */
export type ScriptSide = 'server' | 'vornd'

function lines(text: string): string[] {
  return text
    .split('\n')
    .filter((l) => l.length > 0)
    .sort()
}

/** Every accepted difference applied to one side's run. */
export function normalizeScriptRun(run: ScriptRun, side: ScriptSide): unknown {
  const { result } = run
  const exitedWith = `Exited with code ${result.exitCode}`
  // The server's stderr is its error; on vornd's side the error repeats the output.
  const stderr =
    side === 'server' && result.error && result.error !== exitedWith ? result.error : ''
  return {
    success: result.success,
    exitCode: result.exitCode,
    failed: result.error !== undefined,
    printed: lines(result.output + stderr),
    told: lines(run.told.join('')),
    exits: run.exits
  }
}
