import { useEffect, useState } from 'react'
import { formatDuration, getRunningBlock, onCommandBlocksChange } from '../lib/command-blocks'

/**
 * The command running right now, between the finished blocks and the live
 * terminal.
 *
 * Blocks only appear once a command has finished, so while one is running there
 * was nothing at all to say so — and a command that never exits, `cat` with no
 * arguments being the easy way to get one, looked exactly like a dead terminal.
 * Everything typed afterwards goes to that command's stdin, which reads as the
 * shell having stopped responding.
 *
 * The spine used to carry this. It is not drawn in block mode, since the
 * terminal is cleared after each command and its marks would index nothing.
 */

/** Elapsed time is the whole point, so it has to tick on its own. */
const TICK_MS = 1000

export function RunningCommand({ terminalId }: { terminalId: string }) {
  const [running, setRunning] = useState(() => getRunningBlock(terminalId))
  const [elapsed, setElapsed] = useState('')

  useEffect(() => {
    return onCommandBlocksChange(terminalId, () => setRunning(getRunningBlock(terminalId)))
  }, [terminalId])

  useEffect(() => {
    if (!running) return
    // Read the clock here rather than while rendering: a render that calls
    // Date.now() is not idempotent, and the elapsed value has to survive
    // re-renders it did not cause.
    const update = (): void => setElapsed(formatDuration(Math.max(0, Date.now() - running.since)))
    const first = setTimeout(update, 0)
    const id = setInterval(update, TICK_MS)
    return () => {
      clearTimeout(first)
      clearInterval(id)
    }
  }, [running])

  if (!running) return null

  return (
    <div className="flex shrink-0 items-baseline gap-2 border-t border-white/[0.06] py-1.5 pl-3 pr-3">
      {/* Colour is spent here because this is status: something is happening
          that the rest of the surface cannot show. */}
      <span
        aria-hidden
        className="mt-[6px] h-[6px] w-[6px] shrink-0 animate-pulse rounded-full bg-blue-400"
      />
      <span className="min-w-0 flex-1 truncate font-mono text-[13px] font-semibold text-gray-100">
        {running.command ?? 'Running'}
      </span>
      <span className="shrink-0 font-mono text-[10px] tabular-nums text-white/25">{elapsed}</span>
      <span className="shrink-0 font-mono text-[10px] text-white/25">⌃C to interrupt</span>
    </div>
  )
}
