import type { AppConfig } from '../shared/types'

// Null leaves the OS record alone: a dev build would register node_modules' Electron.
export function loginItemFor(
  config: AppConfig | null | undefined,
  isPackaged: boolean
): boolean | null {
  if (!isPackaged) return null
  if (!config) return null
  return config.defaults?.startAtLogin === true
}
