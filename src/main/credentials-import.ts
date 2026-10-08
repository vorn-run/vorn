/**
 * Hand vornd, once, the secrets this app sealed with its own keychain before
 * vornd kept them in the OS keychain itself: connections' secret fields,
 * stored SSH keys and remote hosts' passwords.
 *
 * A row still holding sealed text is decrypted here and sent with
 * `credentials:import`; vornd files it in its vault and leaves its marker in
 * the row, so the next start finds nothing to hand over.
 */
import { safeStorage } from 'electron'
import { IPC } from '../shared/types'
import type {
  AppConfig,
  ConnectorManifest,
  SSHKey,
  SSHKeyMeta,
  SourceConnection
} from '../shared/types'
import type { ServerBridge } from './server/server-bridge'
import log from './logger'

/** What a row holds where vornd keeps the secret. */
export const IN_VAULT = 'vorn-vault'

interface ConnectorListEntry {
  id: string
  manifest: ConnectorManifest
}

/** The sealed secrets of each connection, decrypted; those that will not decrypt are left out. */
export function sealedSecrets(
  connections: SourceConnection[],
  connectors: ConnectorListEntry[],
  decrypt: (sealed: string) => string | undefined
): Record<string, Record<string, string>> {
  const manifests = new Map(connectors.map((c) => [c.id, c.manifest]))
  const out: Record<string, Record<string, string>> = {}
  for (const conn of connections) {
    const fields = (manifests.get(conn.connectorId)?.auth ?? []).filter(
      (f) => f.type === 'password'
    )
    for (const field of fields) {
      const sealed = (conn.filters as Record<string, unknown>)[field.key]
      if (typeof sealed !== 'string' || sealed === '' || sealed === IN_VAULT) continue
      const plain = decrypt(sealed)
      if (plain === undefined) continue
      ;(out[conn.id] ??= {})[field.key] = plain
    }
  }
  return out
}

/** Each value still sealed, decrypted, by id; those that will not decrypt are left out. */
export function sealedById(
  sealed: Array<[string, string | undefined]>,
  decrypt: (sealed: string) => string | undefined
): Record<string, string> {
  const out: Record<string, string> = {}
  for (const [id, value] of sealed) {
    if (!value || value === IN_VAULT) continue
    const plain = decrypt(value)
    if (plain !== undefined) out[id] = plain
  }
  return out
}

function decryptSealed(sealed: string): string | undefined {
  if (!safeStorage.isEncryptionAvailable()) return undefined
  try {
    return safeStorage.decryptString(Buffer.from(sealed, 'base64'))
  } catch {
    return undefined
  }
}

async function importSealed(bridge: ServerBridge): Promise<void> {
  try {
    const [connections, connectors] = await Promise.all([
      bridge.request<SourceConnection[]>(IPC.CONNECTION_LIST, { connectorId: undefined }),
      bridge.request<ConnectorListEntry[]>(IPC.CONNECTOR_LIST)
    ])
    const [config, keys] = await Promise.all([
      bridge.request<AppConfig>(IPC.CONFIG_LOAD),
      bridge.request<SSHKeyMeta[]>(IPC.CREDENTIAL_LIST_KEYS)
    ])
    const rows = await Promise.all(
      (keys ?? []).map((k) => bridge.request<SSHKey | null>(IPC.CREDENTIAL_GET_ENCRYPTED_KEY, k.id))
    )
    const sealed = sealedSecrets(connections ?? [], connectors ?? [], decryptSealed)
    const sshKeys = sealedById(
      rows.flatMap(
        (row): Array<[string, string]> => (row ? [[row.id, row.encryptedPrivateKey]] : [])
      ),
      decryptSealed
    )
    const hostPasswords = sealedById(
      (config?.remoteHosts ?? []).map((h): [string, string | undefined] => [
        h.id,
        h.encryptedPassword
      ]),
      decryptSealed
    )
    const counts = [sealed, sshKeys, hostPasswords].map((m) => Object.keys(m).length)
    if (counts.every((n) => n === 0)) return
    await bridge.request(IPC.CREDENTIALS_IMPORT, { connections: sealed, sshKeys, hostPasswords })
    log.info(
      `[credentials] handed ${counts[0]} connection(s), ${counts[1]} SSH key(s) and ${counts[2]} host password(s) to the vault`
    )
  } catch (err) {
    log.warn(`[credentials] could not hand sealed secrets to the vault: ${err}`)
  }
}

/** Runs once the bridge is up; a start that finds nothing sealed does nothing. */
export function installCredentialsImport(bridge: ServerBridge): void {
  setTimeout(() => void importSealed(bridge), 500)
}
