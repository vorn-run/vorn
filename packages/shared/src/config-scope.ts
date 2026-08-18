import type { AppConfig } from './types'

/**
 * Which settings belong to the person looking at the screen, rather than to the
 * server they are looking at.
 *
 * Every setting used to live in the server's database and be broadcast to every
 * connected client, so switching from Sessions to Tasks on a laptop switched it on
 * the phone too, and one device's font size was every device's font size. That was
 * invisible while there was one client. Remote access made two normal.
 *
 * The split is by consumer, not by taste. A key belongs to the server when the
 * server or the main process reads it — `shell` and `minimalShellPrompt` are used
 * to spawn a PTY, `networkAccessEnabled` and `serverPort` decide the bind,
 * `updateChannel` drives the updater. Those must stay shared: they describe one
 * machine, and two clients disagreeing about them would be a genuine conflict.
 *
 * A key belongs to the viewer when only the renderer reads it and the answer can
 * differ per device without anything breaking. A phone wanting a bigger font and a
 * different view than the desktop is not a conflict to resolve — it is two correct
 * answers.
 *
 * `defaultAgent` is deliberately not here. It is read in the renderer, but it
 * decides what gets spawned on the server, so it reads as a property of the setup
 * rather than of the viewer.
 */
export const VIEWER_SETTING_KEYS = [
  'mainViewMode',
  'layoutMode',
  'taskViewMode',
  'minimizedPlacement',
  'activeWorkspace',
  'fontSize',
  'rowHeight',
  'theme',
  'notifications',
  'domBlockRendering',
  'enableHoverPreview',
  'showHeadlessAgents',
  'reopenSessions',
  'hasSeenOnboarding'
] as const satisfies ReadonlyArray<keyof AppConfig['defaults']>

export type ViewerSettingKey = (typeof VIEWER_SETTING_KEYS)[number]

export type ViewerSettings = Partial<Pick<AppConfig['defaults'], ViewerSettingKey>>

const VIEWER_KEY_SET: ReadonlySet<string> = new Set(VIEWER_SETTING_KEYS)

export function isViewerSettingKey(key: string): key is ViewerSettingKey {
  return VIEWER_KEY_SET.has(key)
}

/** The viewer-owned subset of a config, for storing on this device. */
export function extractViewerSettings(config: AppConfig): ViewerSettings {
  const out: Record<string, unknown> = {}
  for (const key of VIEWER_SETTING_KEYS) {
    const value = config.defaults[key]
    if (value !== undefined) out[key] = value
  }
  return out as ViewerSettings
}

/**
 * This device's answers laid over the server's.
 *
 * The server's values are the fallback rather than the loser, which is what makes
 * the first run seed itself: a device with nothing stored yet simply keeps showing
 * what the server said, and starts diverging only once someone changes something.
 */
export function applyViewerSettings(config: AppConfig, local: ViewerSettings): AppConfig {
  if (!local || Object.keys(local).length === 0) return config
  return { ...config, defaults: { ...config.defaults, ...local } }
}
