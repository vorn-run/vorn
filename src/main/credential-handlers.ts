import { safeStorage } from 'electron'
import fs from 'node:fs'
import path from 'node:path'
import { safeHandle } from './ipc-safe-handle'
import { IPC } from '../shared/types'
import type { ServerBridge } from './server/server-bridge'

/** SSH keys go to vornd as they are; vornd keeps them in the OS keychain. */
export function registerCredentialHandlers(bridge: ServerBridge): void {
  safeHandle(IPC.CREDENTIAL_SAFE_STORAGE_AVAILABLE, () => {
    return safeStorage.isEncryptionAvailable()
  })

  safeHandle(
    IPC.CREDENTIAL_STORE_KEY,
    (
      _,
      params: {
        label: string
        privateKey: string
        publicKey?: string
        certificate?: string
      }
    ) => bridge.request(IPC.CREDENTIAL_STORE_KEY, params)
  )

  safeHandle(IPC.CREDENTIAL_IMPORT_KEY_FILE, (_, params: { filePath: string; label?: string }) => {
    const privateKey = fs.readFileSync(params.filePath, 'utf-8')
    // The public half, when it sits beside the key as a .pub file.
    const pubPath = params.filePath + '.pub'
    const publicKey = fs.existsSync(pubPath) ? fs.readFileSync(pubPath, 'utf-8').trim() : undefined
    return bridge.request(IPC.CREDENTIAL_STORE_KEY, {
      label: params.label || path.basename(params.filePath),
      privateKey,
      publicKey
    })
  })

  safeHandle(IPC.CREDENTIAL_LIST_KEYS, () => {
    return bridge.request(IPC.CREDENTIAL_LIST_KEYS)
  })

  safeHandle(IPC.CREDENTIAL_DELETE_KEY, (_, id: string) => {
    return bridge.request(IPC.CREDENTIAL_DELETE_KEY, id)
  })
}
