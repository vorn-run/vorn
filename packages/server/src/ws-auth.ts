import crypto from 'node:crypto'
import fs from 'node:fs'
import path from 'node:path'
import { BOOTSTRAP_ENV_VAR, LOCAL_TOKEN_FILENAME } from '@vornrun/shared/protocol'
import { verifyToken, touchLastSeen, constantTimeEqual } from './token-manager'
import { dbGetOwnerUser } from './database'
import log from './logger'

/**
 * Who may open a socket to this server, and how they prove it.
 *
 * The threat this exists for is not a network attacker — it is a *browser*
 * attacker. Browsers apply neither CORS nor same-origin policy to a WebSocket
 * upgrade, and they let a page on any origin open a socket to `localhost`. So
 * binding 127.0.0.1 is no protection at all: before this module, any website the
 * user visited could open `ws://127.0.0.1:<port>/ws` and send `terminal:create`.
 * The ephemeral port is obscurity, not a control — `packages/mcp/src/ws-client.ts`
 * finds it with `lsof` when the port file is missing.
 *
 * Two independent controls, because each covers what the other cannot:
 *
 *   1. An `Origin` allowlist on the upgrade. Browsers always set it and page
 *      script cannot forge it, so this stops the drive-by class outright. It does
 *      nothing against a non-browser client, which can simply omit or lie.
 *   2. A credential on every connection. This is what actually authenticates, and
 *      it is the only control that applies to non-browser clients.
 *
 * There is deliberately no loopback exemption. Loopback is where the attack comes
 * from.
 */

/** How long an unauthenticated socket may stay open before it is closed. */
export const AUTH_TIMEOUT_MS = 10_000

let bootstrapSecret: Buffer | null = null
let localTokenPath: string | null = null

/**
 * Resolve this process's local credential and publish it for same-machine tools.
 *
 * One secret, two delivery channels: the desktop hands it over in the
 * environment, and it is written to `<dataDir>/local-token` (0600) so tools that
 * already read this directory — `packages/mcp`, which opens the SQLite database
 * directly — can present it without new plumbing. A standalone server generates
 * one, so MCP works either way.
 *
 * The file is readable by anything running as this user, which is the same trust
 * boundary `hook-server` already accepts for `~/.vorn/token`. It does not weaken
 * the control that matters here: a hostile web page can open a socket to
 * loopback, but it cannot read a file off disk.
 *
 * Per-process and deleted on shutdown, so nothing usable outlives the server.
 *
 * `publish` is the data directory's ownership, decided once by
 * `claimPublishedFiles` before anything is written. False means another live
 * server already holds this directory, and the secret stays in memory only: the
 * desktop that started this process handed it over in the environment and does
 * not need the file, while anything reading the directory — MCP — is looking for
 * the server that owns it, not for this one. Writing anyway is the bug this
 * closes: a `yarn dev` server overwrote the packaged app's credential and MCP
 * then read one server's port beside another server's secret.
 */
export function initBootstrapSecret(
  dataDir: string,
  value: string | undefined = process.env[BOOTSTRAP_ENV_VAR]
): void {
  pendingDataDir = dataDir
  const supplied = value && value.length > 0 ? value : null
  const secret = supplied ?? crypto.randomBytes(32).toString('base64url')
  bootstrapSecret = Buffer.from(secret, 'utf8')
  if (supplied) log.info('[auth] local credential supplied by the desktop')

  // Remove it from the environment now that it is held in a module variable.
  // `filterEnv` strips the name, but a filter only covers the paths it is asked
  // about — `getUserShellEnv` runs the login shell with the parent environment
  // and reads `env` back, so anything still in `process.env` is reachable. After
  // this, no child can inherit it however it is spawned.
  delete process.env[BOOTSTRAP_ENV_VAR]

  localTokenPath = null
}

/** The directory to publish into, remembered from `initBootstrapSecret`. */
let pendingDataDir: string | null = null

/**
 * Write the credential where same-machine tools will find it.
 *
 * Separate from resolving the secret, and deliberately later, because the two
 * answer different questions. The secret has to exist before any connection can
 * be accepted; the *file* announces this server as the one this machine uses,
 * and that is not true until it has claimed the endpoint.
 *
 * Publishing at startup meant a server that went on to lose the claim had
 * already overwritten the winner's credential, and then exited without a
 * shutdown path to undo it. The winner served with a secret nobody could read
 * and MCP authenticated against a file belonging to a process that no longer
 * existed -- the same failure this whole change is about, arrived at through the
 * race rather than through a dev server.
 */
export function publishLocalCredential(owned: boolean): void {
  const dataDir = pendingDataDir
  const secret = bootstrapSecret?.toString('utf8')
  if (!dataDir || !secret) return
  if (!owned) {
    log.info('[auth] not publishing a local credential: another server owns this directory')
    return
  }

  try {
    localTokenPath = path.join(dataDir, LOCAL_TOKEN_FILENAME)
    // Written fresh each start. `mode` only applies on creation, so remove any
    // existing file first rather than inheriting whatever permissions it had.
    fs.rmSync(localTokenPath, { force: true })
    fs.writeFileSync(localTokenPath, secret, { encoding: 'utf-8', mode: 0o600 })
  } catch (err) {
    localTokenPath = null
    log.warn({ err }, '[auth] could not publish the local credential; MCP will not connect')
  }
}

/**
 * Remove the published credential, if the one on disk is still ours.
 *
 * Two gates. `localTokenPath` is null unless this process published, so a server
 * that stood aside removes nothing. And the content is compared before the
 * unlink, because publishing and shutting down are minutes apart: a file that has
 * since been replaced belongs to whoever replaced it, and deleting it would leave
 * a live server unreachable — the failure this whole change is about, caused on
 * the way out instead of on the way in.
 *
 * Compared by content rather than by a pid beside it: the secret is the one thing
 * this process already knows for certain, and `local-token` holds nothing else —
 * `packages/mcp` reads the whole file as the credential.
 */
export function clearLocalCredential(): void {
  if (!localTokenPath) return
  try {
    if (fs.readFileSync(localTokenPath, 'utf-8') === bootstrapSecret?.toString('utf8')) {
      fs.rmSync(localTokenPath, { force: true })
    }
  } catch {
    /* absent or unreadable — either way there is nothing of ours to remove */
  }
  localTokenPath = null
}

function matchesBootstrap(candidate: string): boolean {
  if (!bootstrapSecret) return false
  return constantTimeEqual(Buffer.from(candidate, 'utf8'), bootstrapSecret)
}

export interface Authenticated {
  userId: string
  /**
   * How the caller proved itself. Three decisions downstream need this and
   * cannot recover it later: only the process holding the bootstrap secret may
   * claim the browser bridge, only a device token can be revoked, and roles will
   * want to tell an interactive user from a same-machine tool.
   */
  kind: 'bootstrap' | 'device'
  /** Absent for the bootstrap credential, which has no database row. */
  tokenId?: string
}

/**
 * Resolve a presented credential, or null.
 *
 * Returning one null for every kind of failure is deliberate: a caller that could
 * distinguish "no such token" from "wrong secret" would leak which ids exist.
 */
export function authenticateCredential(raw: string | undefined): Authenticated | null {
  if (!raw) return null

  // This process's local credential. Checked first because it is the hot path for
  // the desktop and for MCP, and needs no database read.
  if (matchesBootstrap(raw)) {
    const owner = dbGetOwnerUser()
    if (!owner) {
      log.error('[auth] bootstrap credential presented but no owner user exists')
      return null
    }
    return { userId: owner.id, kind: 'bootstrap' }
  }

  const verified = verifyToken(raw)
  if (!verified) return null

  touchLastSeen(verified.tokenId)
  return { userId: verified.userId, kind: 'device', tokenId: verified.tokenId }
}

/** `Authorization: Bearer <token>` → the token, or undefined. */
export function bearerFrom(authorization: string | undefined): string | undefined {
  if (!authorization) return undefined
  const prefix = 'Bearer '
  if (!authorization.startsWith(prefix)) return undefined
  const value = authorization.slice(prefix.length).trim()
  return value.length > 0 ? value : undefined
}
