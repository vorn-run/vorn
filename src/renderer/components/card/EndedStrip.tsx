import { useState } from 'react'
import { Play, Terminal as TerminalIcon, X } from 'lucide-react'
import type { EndedSession } from '../../stores/types'
import { useAppStore } from '../../stores'
import { shortenCwd } from '../../lib/command-blocks'
import { formatRelativeTime } from '../../lib/format-time'
import { resumeEndedSession } from '../../lib/session-resume'
import { closeTerminalSession } from '../../lib/terminal-close'

interface Props {
  terminalId: string
  ended: EndedSession
  /** Grid-card variant: one line, no room for the directory. */
  compact?: boolean
}

/**
 * What a pane says when nothing is running behind it.
 *
 * It stands where the intent bar does, and replaces it rather than stacking
 * above it: a session with no process takes no input, so a composer here would
 * be a control that silently does nothing. That slot is also the only one every
 * surface already has -- the card, the tab body, the focused pane and the
 * terminals panel all render a full-width row under the terminal -- which is how
 * this reaches tab mode, where there is no card header at all.
 *
 * ## Why it is not the accent colour
 *
 * Bronzo means a live piece of work is blocked on a person: a waiting session,
 * an open approval gate. This is not that. A crash ends every pane at once, so
 * the accent would land on the whole screen and stop meaning anything anywhere
 * -- which is the dilution the colour rules warn about. The case for using it is
 * real, because resuming *is* a decision only a person can make; it loses to the
 * fact that this state arrives in bulk and the accent does not survive that.
 */
export function EndedStrip({ terminalId, ended, compact }: Props) {
  const [busy, setBusy] = useState(false)
  const isShell = useAppStore((s) => s.terminals.get(terminalId)?.session.agentType === 'shell')
  const cwd = shortenCwd(ended.cwd ?? null)

  const cause =
    ended.reason === 'exited'
      ? ended.exitCode !== undefined
        ? `exited with ${ended.exitCode}`
        : 'exited'
      : 'the server stopped'

  return (
    <div
      className="flex shrink-0 items-center gap-2.5 border-t border-white/[0.06] py-1.5 pl-3 pr-2"
      role="status"
    >
      {/* A square, not a dot. The dot vocabulary belongs to session status, and
          this is not a status -- it is the absence of one. */}
      <span aria-hidden className="h-[7px] w-[7px] shrink-0 bg-ink-ghost" />

      <span className="min-w-0 flex-1 truncate text-[11px] text-ink-secondary">
        <span className="text-ink">Ended</span>
        <span className="text-ink-faint">
          {' · '}
          {cause}
          {' · '}
          {formatRelativeTime(new Date(ended.at).toISOString())}
        </span>
        {!ended.replayed && (
          <span className="text-ink-faint">{' · nothing of its screen was kept'}</span>
        )}
        {ended.partial && (
          <span className="text-ink-faint">{' · the last moments were not recorded'}</span>
        )}
        {!compact && isShell && cwd && <span className="text-ink-faint">{` · ${cwd}`}</span>}
      </span>

      <button
        onClick={async () => {
          setBusy(true)
          try {
            await resumeEndedSession(terminalId)
          } finally {
            setBusy(false)
          }
        }}
        disabled={busy}
        className="flex shrink-0 items-center gap-1.5 rounded px-2 py-1 text-[11px] text-ink-secondary transition-colors hover:bg-white/[0.08] hover:text-ink disabled:opacity-40"
      >
        {isShell ? <TerminalIcon size={11} /> : <Play size={11} />}
        {isShell ? 'New shell here' : 'Resume'}
      </button>

      <button
        onClick={() => void closeTerminalSession(terminalId)}
        disabled={busy}
        title="Close this pane"
        aria-label="Close this pane"
        className="shrink-0 rounded p-1 text-ink-faint transition-colors hover:bg-white/[0.08] hover:text-ink disabled:opacity-40"
      >
        <X size={11} />
      </button>
    </div>
  )
}
