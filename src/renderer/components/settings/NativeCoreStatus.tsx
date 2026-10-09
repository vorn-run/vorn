import { useEffect, useState } from 'react'
import type { CoreStatus, SessionHolder, SessionHolders, VorndStatus } from '../../../shared/types'

/** Why vornd, which runs every terminal, is not in use, or null. */
function vorndNote(status: VorndStatus | null): string | null {
  if (status?.state !== 'failed') return null
  return `Terminals cannot run, because vornd, the native daemon, is not in use: ${status.detail}.`
}

const plural = (n: number, one: string, many: string): string => `${n} ${n === 1 ? one : many}`

/** What an older session holder means for the sessions on it. */
function olderNote(h: SessionHolder): string {
  const held =
    h.sessions === null
      ? 'Sessions started before Vorn was updated'
      : `${plural(h.sessions, 'session', 'sessions')} started before Vorn was updated`
  const verb = h.sessions === 1 ? 'is' : 'are'
  return h.compatible
    ? `${held} ${verb} still on the older session holder (${h.build}). It exits after the last one ends.`
    : `${held} ${verb} on a session holder (${h.build}) this version cannot talk to. They keep running until you end them.`
}

/** The native core and vornd's session holders: what is wrong with them, and older holders to end. */
export function NativeCoreStatus() {
  // Null until it arrives, and for a server older than the method.
  const [status, setStatus] = useState<CoreStatus | null>(null)
  // Null until it arrives, and for good where vornd's status cannot be asked (the browser).
  const [daemon, setDaemon] = useState<VorndStatus | null>(null)
  const [holders, setHolders] = useState<SessionHolders | null>(null)
  const [endFailure, setEndFailure] = useState<string | null>(null)

  const loadHolders = (isCancelled: () => boolean = () => false): void => {
    void window.api
      .getSessionHolders?.()
      .then((next) => {
        if (!isCancelled()) setHolders(next ?? null)
      })
      .catch(() => {})
  }

  useEffect(() => {
    let cancelled = false
    void window.api
      .getCoreStatus?.()
      .then((next) => {
        if (!cancelled) setStatus(next ?? null)
      })
      .catch(() => {})
    void window.api
      .getVorndStatus?.()
      .then((next) => {
        if (cancelled) return
        setDaemon(next ?? null)
        if (next?.state === 'on') loadHolders(() => cancelled)
      })
      .catch(() => {})
    return () => {
      cancelled = true
    }
  }, [])

  const daemonNote = vorndNote(daemon)

  const endHolder = (h: SessionHolder): void => {
    const count = h.sessions === null ? 'the sessions' : plural(h.sessions, 'session', 'sessions')
    if (!window.confirm(`End ${count} on the older session holder? Their processes stop.`)) return
    void window.api
      .endSessionHolder?.(h.instance)
      .then((outcome) => {
        setEndFailure(outcome.ok ? null : `Could not end them: ${outcome.detail}.`)
        loadHolders()
      })
      .catch((err: Error) => setEndFailure(`Could not end them: ${err.message}.`))
  }

  return (
    <div className="mt-6">
      {status?.version && <div className="text-xs text-gray-500">Native core {status.version}</div>}
      {daemonNote && (
        <div className="mt-2 px-4 py-3 border border-white/[0.08] bg-white/[0.03] rounded-lg text-xs text-gray-400">
          {daemonNote}
        </div>
      )}
      {holders?.error && !holders.current && (
        <div className="mt-2 px-4 py-3 border border-white/[0.08] bg-white/[0.03] rounded-lg text-xs text-gray-400">
          The session holder is not running: {holders.error}.
        </div>
      )}
      {holders?.older
        .filter((h) => h.sessions !== 0)
        .map((h) => (
          <div
            key={h.instance}
            className="mt-2 px-4 py-3 border border-white/[0.08] bg-white/[0.03] rounded-lg text-xs text-gray-400 flex items-center gap-3"
          >
            <span className="flex-1">{olderNote(h)}</span>
            <button
              type="button"
              className="px-2 py-1 rounded border border-white/[0.12] text-gray-300 hover:bg-white/[0.06]"
              onClick={() => endHolder(h)}
            >
              End them
            </button>
          </div>
        ))}
      {endFailure && <div className="mt-2 text-xs text-red-400">{endFailure}</div>}
    </div>
  )
}
