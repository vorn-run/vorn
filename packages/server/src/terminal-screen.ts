import log from './logger'
import {
  nativeCore,
  type NativePipeline,
  type NativeSnapshot,
  type PipelineEvent
} from './native-core'
import { holdPipeline, pipelineFor, pipelineIds, releasePipeline } from './core-pipeline'
import { handBackScrollback, takeScrollback } from './terminal-scrollback'

/**
 * What the terminal currently looks like, as a screen rather than as bytes.
 *
 * `terminal-scrollback` keeps the bytes a terminal emitted, which is what a
 * client needs in order to draw. This keeps what those bytes *mean*: where the
 * cursor is, which modes are set, whether the alternate screen is active, what
 * colour each cell is. Neither can be derived from the other, so both are kept —
 * a checkpoint of a byte buffer is just a smaller byte buffer, while a
 * checkpoint of a screen is a state something can be restored to.
 *
 * Each terminal's model is a libghostty-vt terminal in the Rust core, on a
 * thread of its own (`TerminalPipeline`) that also keeps its scrollback and
 * frames its history. Writes return at once; a read waits for everything
 * written before it. The model never answers a query: the client's xterm is
 * the terminal that does, and a second reply would reach the shell as input.
 *
 * A server without the core keeps no model. Its terminals are still drawn and
 * recorded, from `terminal-scrollback` and the history writer's own framing.
 */

/** A screen, and the things the serialized string does not say. */
export type ScreenSnapshot = NativeSnapshot

/**
 * Feed output to the screen model, for a replay: parsed, but not kept as
 * scrollback or framed for the history. Live output goes through the flush,
 * which hands all three to the pipeline at once.
 *
 * Never throws. A terminal whose model faults is dropped and the session
 * carries on without one. Returns false when the model took the bytes (a bell
 * or a cwd it finds arrives through the reporters below), and null when there
 * is no model.
 */
export function feedScreen(id: string, data: string): boolean | null {
  const pipeline = pipelineFor(id)
  if (!pipeline) return null
  try {
    pipeline.feedScreen(data)
    return false
  } catch (err) {
    drop(id, err)
    return null
  }
}

/**
 * Start modelling a terminal, at the geometry it was spawned with.
 *
 * Explicit rather than created on the first byte, so the geometry comes from the
 * one place that knows it instead of being threaded through every flush.
 */
export function createScreen(
  id: string,
  cols: number,
  rows: number,
  labels?: { title?: string; cwd?: string }
): void {
  clearScreen(id)
  const Pipeline = nativeCore()?.TerminalPipeline
  if (!Pipeline) return
  let pipeline: NativePipeline | undefined
  let before = ''
  try {
    pipeline = new Pipeline(cols, rows, (event) => {
      // A bell rang in output that was live when it was fed, so it counts
      // even from a pipeline that has gone since: an exit feeds the last
      // flush and frees the pipeline in one turn, and its events reach this
      // loop after. Anything else from a pipeline freed or replaced since
      // describes a terminal that is no longer there.
      if (event.kind === 'bell' || pipelineFor(id) === pipeline) pipelineEvent(id, event)
    })
    // A restored screen is rebuilt from escape sequences, and neither label is
    // one, so they are put back beside it.
    if (labels) pipeline.restoreLabels(labels.title, labels.cwd)
    // What this terminal printed before it had a pipeline, so the scrollback
    // carries on rather than starting over.
    before = takeScrollback(id)
    if (before) pipeline.seedScrollback(before)
    holdPipeline(id, pipeline)
  } catch (err) {
    // Not held yet, so dropping it by id would find nothing: stop its thread
    // here and give the scrollback back to the terminal.
    if (pipeline && pipelineFor(id) !== pipeline) {
      if (before) handBackScrollback(id, before)
      try {
        pipeline.free()
      } catch {
        // Already stopped.
      }
    }
    drop(id, err)
  }
}

/**
 * Follow a resize that is not recorded in the history, as a replay's.
 *
 * The values must be the ones node-pty was given, because the program is
 * rendering against those: a model one column wider wraps in a different place,
 * and every line after the first divergence is wrong.
 */
export function resizeScreen(id: string, cols: number, rows: number): void {
  const pipeline = pipelineFor(id)
  if (!pipeline) return
  try {
    pipeline.resize(cols, rows)
  } catch (err) {
    drop(id, err)
  }
}

/** Give up on a session's model. The session itself carries on without one. */
function drop(id: string, err: unknown): void {
  log.warn({ err, id }, '[screen] dropping the screen model for this session')
  clearScreen(id)
}

/**
 * The screen, and what has to travel beside it, once everything fed so far has
 * been parsed. Null when there is no model, or it could not be read.
 */
export async function serializeScreen(id: string): Promise<ScreenSnapshot | null> {
  const pipeline = pipelineFor(id)
  if (!pipeline) return null
  try {
    return pipeline.serialize()
  } catch (err) {
    // Not dropped: the scrollback and the history live in the same
    // pipeline and carry on without a screen.
    log.warn({ err, id }, '[screen] could not serialize')
    return null
  }
}

/**
 * Forget a terminal that has gone. Its thread is stopped now rather than at
 * GC: the terminal's memory is not V8's, so nothing would hurry it.
 */
export function clearScreen(id: string): void {
  const pipeline = releasePipeline(id)
  if (!pipeline) return
  try {
    // Kept for a session resumed under this id; a terminal that exited has
    // already cleared it.
    handBackScrollback(id, pipeline.scrollback())
  } catch {
    // A stopped thread: nothing to hand back.
  }
  try {
    pipeline.free()
  } catch (err) {
    log.warn({ err, id }, '[screen] could not stop a terminal thread')
  }
}

/**
 * Told when a terminal says it has moved.
 *
 * One reporter for the process, set once at start-up, mirroring the renderer's
 * own `setCwdReporter`. It must not throw and must not be slow --
 * `pty-manager`'s does a map lookup and an event.
 */
type CwdReporter = (id: string, cwd: string) => void
let reportCwd: CwdReporter | null = null

export function setCwdReporter(fn: CwdReporter | null): void {
  reportCwd = fn
}

/**
 * Told when a terminal rings its bell. The thread parses after the flush has
 * gone to the clients, so it cannot say so in the flush's answer.
 */
type BellReporter = (id: string) => void
let reportBell: BellReporter | null = null

export function setBellReporter(fn: BellReporter | null): void {
  reportBell = fn
}

function pipelineEvent(id: string, event: PipelineEvent): void {
  switch (event.kind) {
    case 'bell':
      reportBell?.(id)
      return
    case 'cwd':
      if (event.cwd) reportCwd?.(id, event.cwd)
      return
    case 'screen-failed':
      log.warn({ id, error: event.error }, '[screen] the screen model failed; history carries on')
      return
  }
}

/** For the one caller that must not clear a screen recovery has just rebuilt. */
export function hasScreen(id: string): boolean {
  return pipelineFor(id) !== undefined
}

/** How many models are held. For the measurement that bounds this. */
export function screenCount(): number {
  return pipelineIds().length
}

/** Test-only, mirroring `resetScrollback`. */
export function resetScreens(): void {
  for (const id of pipelineIds()) clearScreen(id)
}
