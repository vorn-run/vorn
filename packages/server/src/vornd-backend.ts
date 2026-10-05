import { IPC, type TerminalSession } from '@vornrun/shared/types'
import { clientRegistry } from './broadcast'
import { claimEffect } from './effect-receipts'
import { headlessManager } from './headless-manager'
import log from './logger'
import { nativeDaemonWanted } from './process-backend'
import { ptyManager } from './pty-manager'
import { vorndLink, type LinkEffect } from './vornd-link'
import { setVorndLinkAllowed } from './ws-handler'

/**
 * Puts vornd in as the process backend when it links, and takes what it reports.
 *
 * On every link (vornd started, or restarted by the app) the server lists the
 * sessions vornd's session holder has, takes them on, and then asks vornd to
 * follow: records and effects held while nothing was linked come first, then
 * live ones. Taking them on is what an app restart becomes with the switch on:
 * the terminal list is rebuilt from what is still running, under the same ids,
 * instead of being offered back as restored sessions to resume.
 *
 * Effects (Session Recovery Contract §7): agent status and exits are states the
 * processes apply themselves (`vornd-process.ts`). A desktop notification is
 * shown once per effect id, kept 24 hours in `effect_receipts`; it is the one
 * effect nobody else would act on.
 */
interface BackendDeps {
  /** A session taken on from vornd: tell every window, and save it. */
  adopted(session: TerminalSession): void
}

let current: BackendDeps | null = null

export function wireVorndBackend(deps: BackendDeps): void {
  setVorndLinkAllowed(nativeDaemonWanted)
  clientRegistry.setVorndHeld((id) => ptyManager.isBackendHeld(id))
  // Registering methods again (a test does) replaces the hooks, never doubles them.
  const first = current === null
  current = deps
  if (!first) return

  ptyManager.on('session-adopted', (session: TerminalSession) => current?.adopted(session))

  vorndLink.on('up', () => {
    void takeOn().catch((err) => {
      log.warn({ err }, '[vornd-backend] could not take on the sessions vornd holds')
    })
  })

  // vornd's session holder came back (or another took its place): the ones it
  // did not keep have ended, the way a holder that died ends them.
  vorndLink.on('held', () => {
    void reconcile().catch((err) => {
      log.warn({ err }, '[vornd-backend] could not list the sessions vornd holds')
    })
  })

  vorndLink.on('effect', (effect: LinkEffect) => {
    if (effect.kind !== 'notify') return
    if (!claimEffect(effect.effect, 'notify')) return
    clientRegistry.broadcast(
      IPC.TERMINAL_NOTIFY,
      { id: effect.id, title: effect.title ?? '', body: effect.body ?? '' },
      effect.id
    )
  })
}

async function takeOn(): Promise<void> {
  await reconcile()
  await vorndLink.follow()
  // A client that attached in the gap before vornd held its sessions was
  // answered here; it attaches again, through vornd.
  const held = ptyManager.backendHeldIds()
  clientRegistry.resyncViaVornd(held)
  log.info(
    { terminals: held.length },
    '[vornd-backend] following vornd; clients told to attach again'
  )
}

async function reconcile(): Promise<void> {
  const listing = await vorndLink.list()
  const adopted = ptyManager.adoptBackend(listing)
  headlessManager.adoptBackend(listing)
  log.info(
    { connected: listing.connected, listed: listing.sessions.length, adopted: adopted.length },
    '[vornd-backend] reconciled with vornd'
  )
}
