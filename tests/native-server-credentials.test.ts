/**
 * SSH keys and remote hosts' passwords, kept by vornd in its vault through a
 * real server and the vornd it keeps: a key stored, listed and deleted, a
 * host's password saved with the configuration, and the ones the desktop
 * sealed before handed over once. No row, answer or broadcast holds a secret,
 * and none of these calls reaches the server.
 *
 * Runs where vornd and its session holder have been built.
 */
import fs from 'node:fs'
import path from 'node:path'
import { afterAll, beforeAll, describe, expect, it, vi } from 'vitest'
import type { AppConfig, RemoteHost, SSHKey, SSHKeyMeta } from '../packages/shared/src/types'
import { queryDb } from './helpers/database'
import {
  type RealServer,
  Watcher,
  removeRealServerDirs,
  runnable,
  startRealServer,
  stopRealServer,
  until
} from './helpers/real-server'

vi.setConfig({ testTimeout: 60_000, hookTimeout: 120_000 })

const IN_VAULT = 'vorn-vault'
const KEY = '-----BEGIN OPENSSH PRIVATE KEY----- ED25519 never-in-a-row'
const PASSWORD = 'pw-never-in-a-row'

describe.runIf(runnable)('SSH secrets in vornd', () => {
  let server: RealServer
  let client: Watcher

  const db = (): string => path.join(server.dirs.data, 'vorn.db')
  /** The database's bytes, its write-ahead log included. */
  const dbText = (): string =>
    [db(), `${db()}-wal`]
      .filter((f) => fs.existsSync(f))
      .map((f) => fs.readFileSync(f).toString('latin1'))
      .join('')
  const host = (id: string, extra: Partial<RemoteHost> = {}): RemoteHost => ({
    id,
    label: id,
    hostname: `${id}.example`,
    user: 'me',
    port: 22,
    authMethod: 'password',
    ...extra
  })
  const saveHosts = async (hosts: RemoteHost[]): Promise<AppConfig> => {
    const config = await client.result<AppConfig>('config:load')
    await client.result('config:save', { ...config, remoteHosts: hosts })
    return client.result<AppConfig>('config:load')
  }

  beforeAll(async () => {
    server = await startRealServer()
    client = await Watcher.open(server.vornd)
  })

  afterAll(async () => {
    client?.close()
    if (server) await stopRealServer(server)
    removeRealServerDirs()
  })

  it('stores a key in the vault, lists it, and deletes both', async () => {
    const { id } = await client.result<{ id: string }>('credential:storeKey', {
      label: 'laptop',
      privateKey: KEY,
      publicKey: 'ssh-ed25519 AAAA'
    })
    const row = await client.result<SSHKey>('credential:getEncryptedKey', id)
    expect(row).toMatchObject({
      id,
      label: 'laptop',
      encryptedPrivateKey: IN_VAULT,
      keyType: 'ed25519'
    })
    expect(await client.result<SSHKeyMeta[]>('credential:listKeys')).toEqual([
      expect.objectContaining({ id, label: 'laptop', publicKey: 'ssh-ed25519 AAAA' })
    ])
    expect(dbText()).not.toContain('never-in-a-row')

    await client.result('credential:deleteKey', id)
    expect(await client.result<SSHKeyMeta[]>('credential:listKeys')).toEqual([])
  })

  it('keeps a host’s password out of the configuration', async () => {
    const saved = await saveHosts([host('a', { password: PASSWORD })])
    expect(saved.remoteHosts?.[0]).toMatchObject({ id: 'a', encryptedPassword: IN_VAULT })
    expect(saved.remoteHosts?.[0]).not.toHaveProperty('password')
    expect(JSON.stringify(client.toldBy('config:changed'))).not.toContain(PASSWORD)
    expect(dbText()).not.toContain(PASSWORD)
    // Switched to the agent: the password goes.
    const switched = await saveHosts([host('a', { authMethod: 'agent' })])
    expect(switched.remoteHosts?.[0].encryptedPassword).toBeUndefined()
  })

  it('files once what the desktop sealed, leaving the marker in each row', async () => {
    queryDb(db(), (d) =>
      d
        .prepare(
          'INSERT INTO ssh_keys (id, label, encrypted_private_key, created_at) VALUES (?, ?, ?, ?)'
        )
        .run('sealed-key', 'old', 'c2VhbGVk', new Date().toISOString())
    )
    await saveHosts([host('b', { encryptedPassword: 'c2VhbGVk' })])
    const answer = await client.result('credentials:import', {
      sshKeys: { 'sealed-key': KEY, 'no-such-key': KEY },
      hostPasswords: { b: PASSWORD, 'no-such-host': PASSWORD }
    })
    expect(answer).toEqual({ connections: 0, sshKeys: 1, hostPasswords: 1 })
    const row = await client.result<SSHKey>('credential:getEncryptedKey', 'sealed-key')
    expect(row.encryptedPrivateKey).toBe(IN_VAULT)
    await until('the hosts told', async () => {
      const config = await client.result<AppConfig>('config:load')
      return config.remoteHosts?.[0]?.encryptedPassword === IN_VAULT
    })
    expect(dbText()).not.toContain('never-in-a-row')
  })
})
