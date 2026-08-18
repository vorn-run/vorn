import type { AppConfig } from './types'
import {
  applyViewerSettings,
  extractViewerSettings,
  pickViewerSettings,
  type ViewerSettings
} from './config-scope'

/**
 * Where a device keeps the settings that are its own.
 *
 * `localStorage` rather than a file, because it is the one store both runtimes
 * have: the web client obviously, and the Electron preload, which runs in a
 * Chromium renderer and so has the same API. One implementation, both transports,
 * and it is already scoped per origin — which is exactly per host-plus-device.
 *
 * Every accessor swallows its errors. Private browsing throws on write, and a
 * device that cannot remember its own view preference should still run.
 */
const STORAGE_KEY = 'vorn.viewerSettings'

export function readViewerSettings(): ViewerSettings {
  try {
    const raw = localStorage.getItem(STORAGE_KEY)
    if (!raw) return {}
    // Narrowed to known keys rather than cast: this is device storage, which any
    // script on the origin can write and which outlives the build that wrote it.
    return pickViewerSettings(JSON.parse(raw))
  } catch {
    return {}
  }
}

function writeViewerSettings(settings: ViewerSettings): void {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(settings))
  } catch {
    /* private browsing, or a quota — the session still works, it just forgets */
  }
}

/**
 * Remember this device's answers from a config on its way to the server.
 *
 * Called on save rather than on change, because there is no single place a change
 * happens: thirty-odd call sites each build a whole config and hand it over. The
 * save is where they all meet.
 */
export function captureViewerSettings(config: AppConfig): void {
  writeViewerSettings({ ...readViewerSettings(), ...extractViewerSettings(config) })
}

/**
 * Lay this device's answers over a config arriving from the server.
 *
 * Applied to a load and to every broadcast. The broadcast is the one that matters:
 * without it, another client saving anything at all — a task edit, a connector
 * sync — pushes its view mode into this window.
 */
export function withViewerSettings(config: AppConfig): AppConfig {
  return applyViewerSettings(config, readViewerSettings())
}
