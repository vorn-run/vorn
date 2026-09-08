import { randomBytes } from 'node:crypto'
import type { ExtensionPaneContribution, TerminalSession } from '@vornrun/shared/types'
import { installedPack } from '../connectors/packs'
import { ptyManager } from '../pty-manager'
import { getOrStartHost, tokenFor } from './hosts'
import log from '../logger'

/**
 * An open pane, and what proves a page may speak for it.
 *
 * A page is not given the extension's process token: a page is HTML, and
 * anything it loads would inherit it. What it gets instead is a nonce in its
 * own URL, minted per open and bound to the one session it was opened for, so
 * the authority a page holds ends when its pane closes.
 */

export interface OpenPane {
  extensionId: string
  paneId: string
  sessionId: string
  projectPath: string
  /** Set on a page pane; the URL the app loads. */
  url?: string
  /** Set on a program pane; the terminal drawing it. */
  terminalId?: string
  /** What closes it, and what a page proves itself with. */
  nonce: string
}

const open = new Map<string, OpenPane>()

/** What a nonce entitles, or nothing when it names no open pane. */
export function grantFor(nonce: string): OpenPane | undefined {
  return open.get(nonce)
}

function paneOf(extensionId: string, paneId: string): ExtensionPaneContribution | undefined {
  const pack = installedPack(extensionId)
  if (!pack || pack.kind !== 'extension') return undefined
  return pack.contributes?.panes?.find((pane) => pane.id === paneId)
}

export async function openPane(
  extensionId: string,
  paneId: string,
  session: TerminalSession
): Promise<OpenPane> {
  const pane = paneOf(extensionId, paneId)
  if (!pane) throw new Error(`The extension "${extensionId}" contributes no pane "${paneId}"`)
  const projectPath = session.projectPath
  const worktreePath = session.worktreePath ?? projectPath

  if (pane.command?.length) {
    // Started before the terminal so the program has a bridge to talk to from its first line.
    await getOrStartHost(extensionId, projectPath)
    const token = tokenFor(extensionId, projectPath)
    const [command, ...args] = pane.command
    const terminal = ptyManager.createExtensionPty({
      command,
      args,
      cwd: worktreePath,
      displayName: pane.title,
      env: {
        ...(token && { VORN_EXTENSION_TOKEN: token }),
        VORN_EXTENSION_SESSION: session.id
      }
    })
    const grant: OpenPane = {
      nonce: randomBytes(32).toString('base64url'),
      extensionId,
      paneId,
      sessionId: session.id,
      projectPath,
      terminalId: terminal.id
    }
    open.set(grant.nonce, grant)
    return grant
  }

  if (!pane.web) throw new Error(`The pane "${paneId}" has neither a page nor a program`)
  // A page needs the extension answering before it loads, or its first read fails.
  await getOrStartHost(extensionId, projectPath)
  const nonce = randomBytes(32).toString('base64url')
  const grant: OpenPane = {
    nonce,
    extensionId,
    paneId,
    sessionId: session.id,
    projectPath,
    url: `/extensions/${extensionId}/pane/${paneId}/${nonce}/`
  }
  open.set(nonce, grant)
  return grant
}

/** Closing takes the authority with it, whichever end asks. */
export function closePane(nonce: string): boolean {
  const grant = open.get(nonce)
  if (!grant) return false
  open.delete(nonce)
  if (grant.terminalId) {
    try {
      ptyManager.killPty(grant.terminalId)
    } catch (err) {
      log.warn(`[extensions] closing the ${grant.paneId} terminal failed: ${err}`)
    }
  }
  return true
}

export function closePanesForSession(sessionId: string): void {
  for (const grant of [...open.values()]) {
    if (grant.sessionId === sessionId) closePane(grant.nonce)
  }
}

export function closePanesForExtension(extensionId: string): void {
  for (const grant of [...open.values()]) {
    if (grant.extensionId === extensionId) closePane(grant.nonce)
  }
}

export function openPanesForSession(sessionId: string): OpenPane[] {
  return [...open.values()].filter((grant) => grant.sessionId === sessionId)
}
