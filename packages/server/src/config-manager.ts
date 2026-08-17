import fs from 'node:fs'
import { AppConfig } from '@vornrun/shared/types'
import { DEFAULT_AGENT_COMMANDS } from '@vornrun/shared/agent-defaults'
import {
  initDatabase,
  closeDatabase,
  getDataDir,
  loadConfig as dbLoadConfig,
  saveConfig as dbSaveConfig
} from './database'
import log from './logger'
import { getDefaultShell } from './process-utils'

type ConfigChangeCallback = (config: AppConfig) => void

class ConfigManager {
  private changeCallbacks: ConfigChangeCallback[] = []
  private dbWatcher: fs.FSWatcher | null = null
  private debounceTimer: ReturnType<typeof setTimeout> | null = null
  private cachedConfig: AppConfig | null = null

  init(dataDir?: string): void {
    initDatabase(dataDir)
  }

  close(): void {
    this.stopWatchingDb()
    closeDatabase()
  }

  loadConfig(): AppConfig {
    if (this.cachedConfig) return this.cachedConfig
    try {
      const config = dbLoadConfig()
      this.cachedConfig = config
      return config
    } catch (err) {
      log.error('[config-manager] loadConfig failed, returning defaults:', err)
      return {
        version: 1,
        defaults: {
          shell: getDefaultShell(),
          fontSize: 13,
          theme: 'dark'
        },
        projects: [],
        agentCommands: { ...DEFAULT_AGENT_COMMANDS },
        workflows: [],
        tasks: []
      }
    }
  }

  saveConfig(config: AppConfig): void {
    try {
      dbSaveConfig(config)
      this.cachedConfig = null
    } catch (err) {
      log.error('[config-manager] saveConfig failed:', err)
      throw err
    }
  }

  /** Register a callback for when config changes from within the main process */
  onConfigChanged(callback: ConfigChangeCallback): void {
    this.changeCallbacks.push(callback)
  }

  /** Notify all registered callbacks (call after main-process config mutations) */
  notifyChanged(): void {
    this.cachedConfig = null
    const config = this.loadConfig()
    for (const cb of this.changeCallbacks) {
      cb(config)
    }
  }

  /**
   * Watch for external DB writes (e.g. MCP stdio process).
   * Detects: .db-signal (explicit), .db-wal changes, and .db changes (post-checkpoint).
   */
  watchDb(): void {
    if (this.dbWatcher) return

    const WATCH_SUFFIXES = ['.db-signal', '.db-wal', '.db']

    try {
      // Whatever directory the database actually landed in, not a second copy
      // of the default — a server on a custom --data-dir must watch its own.
      this.dbWatcher = fs.watch(getDataDir(), (eventType, filename) => {
        if (!filename || !WATCH_SUFFIXES.some((s) => filename.endsWith(s))) return
        // Debounce -- multiple writes can fire rapidly
        if (this.debounceTimer) clearTimeout(this.debounceTimer)
        this.debounceTimer = setTimeout(() => {
          this.notifyChanged()
        }, 300)
      })
    } catch {
      // Directory may not exist yet; ignore
    }
  }

  private stopWatchingDb(): void {
    if (this.dbWatcher) {
      this.dbWatcher.close()
      this.dbWatcher = null
    }
    if (this.debounceTimer) {
      clearTimeout(this.debounceTimer)
      this.debounceTimer = null
    }
  }

  // No-ops -- retained for API compatibility during transition
  watchConfig(_callback: ConfigChangeCallback): void {
    /* no-op with SQLite */
  }
  stopWatching(): void {
    /* no-op with SQLite */
  }
}

export const configManager = new ConfigManager()
