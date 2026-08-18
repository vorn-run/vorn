import fs from 'node:fs'
import path from 'node:path'
import { app, safeStorage } from 'electron'
import log from '../logger'

/**
 * Which server this desktop talks to, remembered locally.
 *
 * This is the first thing Vorn stores outside the server's database, and it has to
 * be: everything else lives in that database, and host mode has to know which
 * database to open before it has one. Asking config where the host is would mean
 * connecting to the host to find out where the host is.
 *
 * The credential vault is no use here for the same reason — `registerCredentialHandlers`
 * takes a live bridge, and every path in it persists ciphertext server-side. Only
 * the encryption primitive is reusable, so that is all this borrows.
 */
export type ServerMode = 'local' | 'host'

export interface HostSettings {
  mode: ServerMode
  /** Always `ws://host:port/ws` or `wss://…`, as `normaliseHostUrl` produces. Empty in local mode. */
  url: string
  /** Present only when a token has been stored and safeStorage can decrypt it. */
  token?: string
}

const FILE = (): string => path.join(app.getPath('userData'), 'host.json')

interface StoredShape {
  mode?: string
  url?: string
  /** base64 of a safeStorage blob. Never the plaintext. */
  encryptedToken?: string
}

const LOCAL: HostSettings = { mode: 'local', url: '' }

/**
 * A device token is equivalent to a shell on the host it points at, so it is
 * encrypted at rest with the OS keychain the SSH keys already use.
 *
 * When encryption is unavailable — a Linux box with no keyring — the token is not
 * written at all rather than written in the clear. Host mode then asks for it each
 * launch, which is worse to use and better than leaving a shell credential in a
 * plaintext file under the user's home directory.
 */
export function readHostSettings(): HostSettings {
  let raw: string
  try {
    raw = fs.readFileSync(FILE(), 'utf-8')
  } catch {
    return LOCAL
  }

  let parsed: StoredShape
  try {
    parsed = JSON.parse(raw) as StoredShape
  } catch {
    log.warn('[host-store] host.json is unreadable; falling back to a local server')
    return LOCAL
  }

  if (parsed.mode !== 'host' || !parsed.url) return LOCAL

  let token: string | undefined
  if (parsed.encryptedToken) {
    try {
      token = safeStorage.decryptString(Buffer.from(parsed.encryptedToken, 'base64'))
    } catch (err) {
      // A keychain that has moved, or a file copied between machines. The host is
      // still known; the credential has to be entered again.
      log.warn({ err }, '[host-store] stored token could not be decrypted')
    }
  }

  return { mode: 'host', url: parsed.url, token }
}

export function writeHostSettings(settings: HostSettings): void {
  const stored: StoredShape = { mode: settings.mode, url: settings.url }

  if (settings.mode === 'host' && settings.token) {
    if (safeStorage.isEncryptionAvailable()) {
      stored.encryptedToken = safeStorage.encryptString(settings.token).toString('base64')
    } else {
      log.warn('[host-store] no OS encryption available; the token will not be remembered')
    }
  }

  try {
    const file = FILE()
    fs.writeFileSync(file, JSON.stringify(stored, null, 2), { encoding: 'utf-8', mode: 0o600 })
    // `mode` above only applies when the file is created, so a host.json left
    // world-readable by an older build or a hand edit would keep those permissions
    // for good. It holds an encrypted credential, so the mode is asserted on every
    // write rather than hoped for.
    fs.chmodSync(file, 0o600)
  } catch (err) {
    log.error({ err }, '[host-store] could not write host.json')
  }
}

/** Return to running a server on this machine, and forget the stored credential. */
export function clearHostSettings(): void {
  try {
    fs.rmSync(FILE(), { force: true })
  } catch (err) {
    log.error({ err }, '[host-store] could not clear host.json')
  }
}
