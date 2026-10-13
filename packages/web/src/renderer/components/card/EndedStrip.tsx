import { useState, type ReactElement } from 'react'
import { Play, Terminal as TerminalIcon, X } from 'lucide-react'
import type { EndedSession } from '../../stores/types'
import { useAppStore } from '../../stores'
import { headMoved } from '../../../shared/types'
import { shortenCwd } from '../../lib/command-blocks'
import { formatRelativeTime } from '../../lib/format-time'
import { resumeEndedSession } from '../../lib/session-resume'
import { closeTerminalSession } from '../../lib/terminal-close'
import { readIntentDraft, forgetIntentDraft } from '../../lib/intent-drafts'
import { pasteToTerminal } from '../../lib/terminal-registry'

interface Props {
  terminalId: string
  ended: EndedSession
  /** Grid-card variant: one line, no room for the directory. */
  compact?: boolean
}

const sha = (commit: string): string => commit.slice(0, 8)

function formatSpan(ms: number): string {
  const mins = Math.max(0, Math.floor(ms / 60_000))
  if (mins < 60) return `${mins}m`
  return `${Math.floor(mins / 60)}h ${mins % 60}m`
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
 *
 * ## After a reboot
 *
 * The record was checked against the tree before it was offered. A worktree
 * that is gone gets a summary and no Resume, because there is nowhere to resume
 * into. A HEAD that moved gets both commits and a warning, never a checkout:
 * the tree is the person's, not the restore's. A shell command typed and not
 * sent is offered back, and never run on its own.
 */
export function EndedStrip({ terminalId, ended, compact }: Props) {
  const [busy, setBusy] = useState(false)
  const [warningLeft, setWarningLeft] = useState(false)
  const session = useAppStore((s) => s.terminals.get(terminalId)?.session)
  const isShell = session?.agentType === 'shell'
  const cwd = shortenCwd(ended.cwd ?? null)
  const [draft, setDraft] = useState(() =>
    isShell ? readIntentDraft(terminalId)?.text : undefined
  )

  const env = ended.environment
  const worktreeGone = env?.worktree === 'missing'
  const moved = headMoved(env)
  const finding = worktreeGone ? 'the worktree is gone' : moved ? 'the branch moved' : null

  // Four endings, and telling them apart is most of the value here. Reporting
  // an ordinary quit as though something had stopped the server reads like a
  // fault report for closing an app.
  const cause =
    ended.reason === 'exited'
      ? ended.exitCode !== undefined
        ? `exited with ${ended.exitCode}`
        : 'exited'
      : ended.reason === 'app-closed'
        ? 'Vorn was closed'
        : ended.reason === 'machine-restarted'
          ? 'the machine restarted'
          : 'the server stopped unexpectedly'

  const resume = async (): Promise<void> => {
    setBusy(true)
    try {
      await resumeEndedSession(terminalId)
    } finally {
      setBusy(false)
    }
  }

  // Two decisions, made with two clicks: resuming is not agreeing to send.
  const resumeAndRun = async (): Promise<void> => {
    const text = draft
    await resume()
    if (!text || useAppStore.getState().terminals.get(terminalId)?.ended) return
    forgetIntentDraft(terminalId)
    setDraft(undefined)
    setTimeout(() => {
      pasteToTerminal(terminalId, text)
      window.api.writeTerminal(terminalId, '\r')
    }, 300)
  }

  const discardDraft = (): void => {
    forgetIntentDraft(terminalId)
    setDraft(undefined)
  }

  const showWhatChanged = (): void => {
    if (!env || !env.head.recorded || !env.head.actual) return
    useAppStore.getState().setDiffSidebarTerminalId(terminalId, 'changes', {
      from: env.head.recorded,
      to: env.head.actual
    })
  }

  const warning = moved && !worktreeGone && !warningLeft
  const offerDraft = !!draft && !worktreeGone && !warning
  const showChecks =
    !compact && env && !isShell && (ended.reason === 'machine-restarted' || finding)
  const resumeInRow = !worktreeGone && !warning && !offerDraft

  const check = (ok: boolean, label: string, value: string): ReactElement => (
    <div className="flex gap-2 text-[11px]">
      <span className={ok ? 'text-ink-faint' : 'text-red-400'} aria-hidden>
        {ok ? '✓' : '✗'}
      </span>
      <span className="w-14 shrink-0 text-ink-faint">{label}</span>
      <span className={`min-w-0 truncate ${ok ? 'text-ink-secondary' : 'text-ink'}`}>{value}</span>
    </div>
  )

  const button = (label: string, onClick: () => void, icon?: ReactElement): ReactElement => (
    <button
      key={label}
      onClick={onClick}
      disabled={busy}
      className="flex shrink-0 items-center gap-1.5 rounded px-2 py-1 text-[11px] text-ink-secondary transition-colors hover:bg-white/[0.08] hover:text-ink disabled:opacity-40"
    >
      {icon}
      {label}
    </button>
  )

  return (
    <div className="flex shrink-0 flex-col border-t border-white/[0.06]" role="status">
      <div className="flex items-center gap-2.5 py-1.5 pl-3 pr-2">
        {/* A square, not a dot. The dot vocabulary belongs to session status, and
            this is not a status -- it is the absence of one. */}
        <span
          aria-hidden
          className={`h-[7px] w-[7px] shrink-0 ${finding ? 'bg-red-400' : 'bg-ink-ghost'}`}
        />

        <span className="min-w-0 flex-1 truncate text-[11px] text-ink-secondary">
          <span className="text-ink">Ended</span>
          <span className="text-ink-faint">
            {' · '}
            {cause}
            {' · '}
            {finding ?? formatRelativeTime(new Date(ended.at).toISOString())}
          </span>
          {!ended.replayed && (
            <span className="text-ink-faint">{' · nothing of its screen was kept'}</span>
          )}
          {ended.partial && (
            <span className="text-ink-faint">{' · the last moments were not recorded'}</span>
          )}
          {!compact && isShell && cwd && <span className="text-ink-faint">{` · ${cwd}`}</span>}
        </span>

        {resumeInRow &&
          button(
            isShell ? 'New shell here' : 'Resume',
            () => void resume(),
            isShell ? <TerminalIcon size={11} /> : <Play size={11} />
          )}

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

      {showChecks && env && (
        <div className="flex flex-col gap-0.5 px-3 pb-2">
          {check(
            !worktreeGone,
            'worktree',
            `${shortenCwd(session?.worktreePath ?? session?.projectPath ?? null) ?? ''}${worktreeGone ? ' — missing' : ''}`
          )}
          {!worktreeGone &&
            check(
              env.branch.actual === null || env.branch.recorded === env.branch.actual,
              'branch',
              env.branch.actual ?? env.branch.recorded ?? '—'
            )}
          {!worktreeGone &&
            env.head.recorded &&
            check(
              !moved,
              'HEAD',
              moved && env.head.actual
                ? `was ${sha(env.head.recorded)} · now ${sha(env.head.actual)}`
                : `${sha(env.head.recorded)} · unchanged`
            )}
        </div>
      )}

      {/* A summary, not an error. Says what the session was, which is what is left. */}
      {worktreeGone && session && !compact && (
        <div className="flex flex-col gap-1 px-3 pb-2 text-[11px] text-ink-secondary">
          <div className="flex gap-2">
            <span className="w-14 shrink-0 text-ink-faint">agent</span>
            <span>{session.agentType}</span>
          </div>
          {session.branch && (
            <div className="flex gap-2">
              <span className="w-14 shrink-0 text-ink-faint">branch</span>
              <span className="truncate">{session.branch}</span>
            </div>
          )}
          {session.displayName && (
            <div className="flex gap-2">
              <span className="w-14 shrink-0 text-ink-faint">last turn</span>
              <span className="truncate">{session.displayName}</span>
            </div>
          )}
          <div className="flex gap-2">
            <span className="w-14 shrink-0 text-ink-faint">ran</span>
            <span>{formatSpan(ended.at - session.createdAt)}</span>
          </div>
          <p className="mt-1 text-ink-faint">
            Cleaned up after it merged, most likely. Nothing to resume into. The conversation is
            still readable, and this pane can be closed.
          </p>
        </div>
      )}

      {/* A warning, not a lock. Never resolved by checking anything out. */}
      {warning && (
        <div className="flex flex-col gap-1.5 px-3 pb-2">
          <p className="text-[11px] text-ink-secondary">
            Something changed the tree while you were away. Resuming would put an agent back into a
            conversation about a tree that no longer matches it.
          </p>
          <div className="flex gap-1">
            {button('Resume anyway', () => void resume(), <Play size={11} />)}
            {button('Show what changed', showWhatChanged)}
            {button('Leave it', () => setWarningLeft(true))}
          </div>
        </div>
      )}

      {/* Offered back, never run on its own. */}
      {offerDraft && (
        <div className="flex flex-col gap-1.5 px-3 pb-2">
          <span className="text-[11px] text-ink-faint">You were writing</span>
          <pre className="max-h-24 overflow-auto whitespace-pre-wrap break-words font-mono text-[11px] text-ink">
            {draft}
          </pre>
          <div className="flex gap-1">
            {button('New shell and run it', () => void resumeAndRun(), <TerminalIcon size={11} />)}
            {button('New shell without it', () => void resume())}
            {button('Discard', discardDraft)}
          </div>
        </div>
      )}
    </div>
  )
}
