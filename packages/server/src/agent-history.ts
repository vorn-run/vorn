import Database from 'libsql'
import fs from 'node:fs'
import path from 'node:path'
import os from 'node:os'
import {
  AiAgentType,
  RecentSession,
  getRecentSessionActivityLabel,
  supportsExactSessionResume
} from '@vornrun/shared/types'
import { listWorktrees } from './git-utils'
import { normalizePath } from './process-utils'

interface AgentHistoryProvider {
  getRecentSessions(scope?: ProjectScope, limit?: number): RecentSession[]
}

interface ProjectScope {
  rawPaths: string[]
  normalizedPaths: Set<string>
}

// ---------------------------------------------------------------------------
// Shared helper -- run a read-only SQLite query via libsql (works on Windows)
// ---------------------------------------------------------------------------

function querySqlite(dbPath: string, sql: string): Record<string, unknown>[] {
  try {
    const db = new Database(dbPath, { readonly: true })
    try {
      return db.prepare(sql).all() as Record<string, unknown>[]
    } finally {
      try {
        db.close()
      } catch {
        // Swallow close errors so a successful query result isn't discarded.
      }
    }
  } catch {
    return []
  }
}

function escapeSqlString(value: string): string {
  return value.replace(/'/g, "''")
}

function buildPathWhereClause(column: string, scope: ProjectScope): string {
  const clauses = scope.rawPaths.map((projectPath) => {
    const normalized = normalizePath(projectPath).replace(/\\/g, '/').toLowerCase()
    return `lower(rtrim(replace(${column}, char(92), '/'), '/')) = '${escapeSqlString(normalized)}'`
  })
  return clauses.length === 1 ? clauses[0] : `(${clauses.join(' OR ')})`
}

function createProjectScope(projectPath?: string): ProjectScope | undefined {
  if (!projectPath) return undefined

  const normalizedPaths = new Set<string>()
  const rawPaths: string[] = []
  const addPath = (candidatePath: string): void => {
    const normalized = normalizePath(candidatePath)
    if (normalizedPaths.has(normalized)) return
    normalizedPaths.add(normalized)
    rawPaths.push(candidatePath)
  }

  addPath(projectPath)
  for (const worktree of listWorktrees(projectPath)) {
    addPath(worktree.path)
  }

  return { rawPaths, normalizedPaths }
}

function filterSessionsByProjectPath(
  sessions: RecentSession[],
  scope?: ProjectScope
): RecentSession[] {
  if (!scope) return sessions

  const normalizedCache = new Map<string, string>()
  const normalizeSessionPath = (value: string): string => {
    let normalized = normalizedCache.get(value)
    if (normalized === undefined) {
      normalized = normalizePath(value)
      normalizedCache.set(value, normalized)
    }
    return normalized
  }

  return sessions.filter((session) =>
    scope.normalizedPaths.has(normalizeSessionPath(session.projectPath))
  )
}

function getOpenCodeDbPath(): string {
  const baseDir =
    process.platform === 'win32'
      ? process.env.LOCALAPPDATA || path.join(os.homedir(), 'AppData', 'Local')
      : process.env.XDG_DATA_HOME || path.join(os.homedir(), '.local', 'share')
  return path.join(baseDir, 'opencode', 'opencode.db')
}

// ---------------------------------------------------------------------------
// Claude Code  --  ~/.claude/history.jsonl
// ---------------------------------------------------------------------------

interface ClaudeHistoryEntry {
  sessionId: string
  display: string
  project: string
  timestamp: number
}

const claudeProvider: AgentHistoryProvider = {
  getRecentSessions(scope?: ProjectScope, limit = 20): RecentSession[] {
    const historyPath = path.join(os.homedir(), '.claude', 'history.jsonl')

    try {
      if (!fs.existsSync(historyPath)) return []

      const raw = fs.readFileSync(historyPath, 'utf-8')
      const lines = raw.trim().split('\n').filter(Boolean)

      // Normalize filter once; cache per-entry results so repeated project
      // paths (common in history) only hit realpathSync once.
      const normalizedCache = new Map<string, string>()
      const normalizeEntry = (p: string): string => {
        let cached = normalizedCache.get(p)
        if (cached === undefined) {
          cached = normalizePath(p)
          normalizedCache.set(p, cached)
        }
        return cached
      }

      const sessionMap = new Map<
        string,
        {
          display: string
          projectPath: string
          firstTimestamp: number
          lastTimestamp: number
          count: number
        }
      >()

      for (const line of lines) {
        try {
          const entry: ClaudeHistoryEntry = JSON.parse(line)
          if (!entry.sessionId || !entry.project) continue

          if (scope && !scope.normalizedPaths.has(normalizeEntry(entry.project))) continue

          const existing = sessionMap.get(entry.sessionId)
          if (existing) {
            existing.lastTimestamp = Math.max(existing.lastTimestamp, entry.timestamp)
            existing.count++
          } else {
            sessionMap.set(entry.sessionId, {
              display: entry.display || '',
              projectPath: entry.project,
              firstTimestamp: entry.timestamp,
              lastTimestamp: entry.timestamp,
              count: 1
            })
          }
        } catch {
          // skip malformed lines
        }
      }

      return Array.from(sessionMap.entries())
        .map(([sessionId, data]) => ({
          sessionId,
          agentType: 'claude' as AiAgentType,
          display: data.display,
          projectPath: data.projectPath,
          timestamp: data.lastTimestamp,
          activityCount: data.count,
          activityLabel: getRecentSessionActivityLabel('claude'),
          canResumeExact: supportsExactSessionResume('claude')
        }))
        .sort((a, b) => b.timestamp - a.timestamp)
        .slice(0, limit)
    } catch {
      return []
    }
  }
}

// ---------------------------------------------------------------------------
// Gemini CLI  --  ~/.gemini/projects.json + ~/.gemini/tmp/*/chats/session-*.json
// ---------------------------------------------------------------------------

interface GeminiSessionFile {
  sessionId: string
  startTime: string
  lastUpdated: string
  messages: { id: string; timestamp: string; type: string; content: string | unknown }[]
}

interface GeminiProjectSource {
  projectPath: string
  chatsDir: string
}

function extractGeminiTextContent(value: unknown): string {
  if (typeof value === 'string') return value
  if (Array.isArray(value)) {
    return value
      .map((part) => extractGeminiTextContent(part))
      .filter(Boolean)
      .join(' ')
      .trim()
  }
  if (value && typeof value === 'object') {
    const record = value as Record<string, unknown>
    if (typeof record.text === 'string') return record.text
    if ('content' in record) return extractGeminiTextContent(record.content)
    if ('parts' in record) return extractGeminiTextContent(record.parts)
  }
  return ''
}

function discoverGeminiProjectSources(geminiDir: string): GeminiProjectSource[] {
  const tmpDir = path.join(geminiDir, 'tmp')
  const projectsPath = path.join(geminiDir, 'projects.json')
  const sources: GeminiProjectSource[] = []
  const seen = new Set<string>()

  const addSource = (projectPath: string, chatsDir: string): void => {
    if (!projectPath || !fs.existsSync(chatsDir)) return
    const key = `${normalizePath(projectPath)}::${chatsDir}`
    if (seen.has(key)) return
    seen.add(key)
    sources.push({ projectPath, chatsDir })
  }

  if (fs.existsSync(tmpDir)) {
    for (const entry of fs.readdirSync(tmpDir)) {
      const projectDir = path.join(tmpDir, entry)
      const rootMarker = path.join(projectDir, '.project_root')
      if (!fs.existsSync(rootMarker)) continue

      try {
        const projectPath = fs.readFileSync(rootMarker, 'utf-8').trim()
        addSource(projectPath, path.join(projectDir, 'chats'))
      } catch {
        // skip unreadable project root markers
      }
    }
  }

  if (fs.existsSync(projectsPath)) {
    try {
      const projectsData = JSON.parse(fs.readFileSync(projectsPath, 'utf-8')) as {
        projects: Record<string, string>
      }
      for (const [projectPath, projectName] of Object.entries(projectsData.projects)) {
        addSource(projectPath, path.join(tmpDir, projectName, 'chats'))
      }
    } catch {
      // skip malformed projects.json
    }
  }

  return sources
}

const geminiProvider: AgentHistoryProvider = {
  getRecentSessions(scope?: ProjectScope, limit = 20): RecentSession[] {
    const geminiDir = path.join(os.homedir(), '.gemini')

    try {
      const projectsToScan = discoverGeminiProjectSources(geminiDir).filter(
        ({ projectPath }) => !scope || scope.normalizedPaths.has(normalizePath(projectPath))
      )

      if (scope && projectsToScan.length === 0) {
        return []
      }

      const sessions: RecentSession[] = []

      for (const { projectPath, chatsDir } of projectsToScan) {
        const files = fs
          .readdirSync(chatsDir)
          .filter((f) => f.startsWith('session-') && f.endsWith('.json'))
          .map((f) => ({ name: f, mtime: fs.statSync(path.join(chatsDir, f)).mtimeMs }))
          .sort((a, b) => b.mtime - a.mtime)
          .slice(0, limit)

        for (const file of files) {
          try {
            const raw = fs.readFileSync(path.join(chatsDir, file.name), 'utf-8')
            const session: GeminiSessionFile = JSON.parse(raw)

            const userMessages = session.messages.filter((m) => m.type === 'user')
            const firstMsg = userMessages[0]?.content
            const display = extractGeminiTextContent(firstMsg).slice(0, 80)

            sessions.push({
              sessionId: session.sessionId,
              agentType: 'gemini',
              display,
              projectPath,
              timestamp: new Date(session.lastUpdated).getTime(),
              activityCount: userMessages.length,
              activityLabel: getRecentSessionActivityLabel('gemini'),
              canResumeExact: supportsExactSessionResume('gemini')
            })
          } catch {
            // skip malformed session files
          }
        }
      }

      return sessions.sort((a, b) => b.timestamp - a.timestamp).slice(0, limit)
    } catch {
      return []
    }
  }
}

// ---------------------------------------------------------------------------
// Codex CLI  --  ~/.codex/state_5.sqlite (threads) + ~/.codex/history.jsonl
// ---------------------------------------------------------------------------

const codexProvider: AgentHistoryProvider = {
  getRecentSessions(scope?: ProjectScope, limit = 20): RecentSession[] {
    const dbPath = path.join(os.homedir(), '.codex', 'state_5.sqlite')

    try {
      if (!fs.existsSync(dbPath)) return []

      // Build message count map from history.jsonl
      const msgCounts = new Map<string, number>()
      const historyPath = path.join(os.homedir(), '.codex', 'history.jsonl')
      if (fs.existsSync(historyPath)) {
        const lines = fs.readFileSync(historyPath, 'utf-8').trim().split('\n').filter(Boolean)
        for (const line of lines) {
          try {
            const entry = JSON.parse(line) as { session_id?: string }
            if (entry.session_id) {
              msgCounts.set(entry.session_id, (msgCounts.get(entry.session_id) || 0) + 1)
            }
          } catch {
            /* skip */
          }
        }
      }

      const mapRows = (rows: Record<string, unknown>[]): RecentSession[] =>
        rows.map((row) => ({
          sessionId: String(row.id),
          agentType: 'codex' as AiAgentType,
          display: String(row.title || row.first_user_message || '').slice(0, 80),
          projectPath: String(row.cwd || ''),
          timestamp: Number(row.updated_at) * 1000,
          activityCount: msgCounts.get(String(row.id)) || 1,
          activityLabel: getRecentSessionActivityLabel('codex'),
          canResumeExact: supportsExactSessionResume('codex')
        }))

      const baseSql =
        'SELECT id, cwd, title, updated_at, first_user_message FROM threads WHERE archived = 0'
      const scopedSql = scope
        ? `${baseSql} AND ${buildPathWhereClause('cwd', scope)} ORDER BY updated_at DESC LIMIT ${limit}`
        : `${baseSql} ORDER BY updated_at DESC LIMIT ${limit}`

      let sessions = filterSessionsByProjectPath(mapRows(querySqlite(dbPath, scopedSql)), scope)
      if (scope && sessions.length === 0) {
        const fallbackSql = `${baseSql} ORDER BY updated_at DESC`
        sessions = filterSessionsByProjectPath(mapRows(querySqlite(dbPath, fallbackSql)), scope)
      }

      return sessions.slice(0, limit)
    } catch {
      return []
    }
  }
}

// ---------------------------------------------------------------------------
// GitHub Copilot CLI  --  ~/.copilot/session-store.db (sessions + turns)
// ---------------------------------------------------------------------------

const copilotProvider: AgentHistoryProvider = {
  getRecentSessions(scope?: ProjectScope, limit = 20): RecentSession[] {
    const dbPath = path.join(os.homedir(), '.copilot', 'session-store.db')

    try {
      if (!fs.existsSync(dbPath)) return []

      const mapRows = (rows: Record<string, unknown>[]): RecentSession[] =>
        rows.map((row) => ({
          sessionId: String(row.id),
          agentType: 'copilot' as AiAgentType,
          display: String(row.summary || '').slice(0, 80),
          projectPath: String(row.cwd || ''),
          timestamp: new Date(String(row.updated_at)).getTime(),
          activityCount: Number(row.turn_count) || 0,
          activityLabel: getRecentSessionActivityLabel('copilot'),
          canResumeExact: supportsExactSessionResume('copilot')
        }))

      const baseSql =
        'SELECT s.id, s.cwd, s.summary, s.updated_at, COUNT(t.id) as turn_count FROM sessions s LEFT JOIN turns t ON s.id = t.session_id'
      const scopedSql = scope
        ? `${baseSql} WHERE ${buildPathWhereClause('s.cwd', scope)} GROUP BY s.id ORDER BY s.updated_at DESC LIMIT ${limit}`
        : `${baseSql} GROUP BY s.id ORDER BY s.updated_at DESC LIMIT ${limit}`

      let sessions = filterSessionsByProjectPath(mapRows(querySqlite(dbPath, scopedSql)), scope)
      if (scope && sessions.length === 0) {
        const fallbackSql = `${baseSql} GROUP BY s.id ORDER BY s.updated_at DESC`
        sessions = filterSessionsByProjectPath(mapRows(querySqlite(dbPath, fallbackSql)), scope)
      }

      return sessions.slice(0, limit)
    } catch {
      return []
    }
  }
}

// ---------------------------------------------------------------------------
// OpenCode  --  XDG/LocalAppData opencode.db (session + message)
// ---------------------------------------------------------------------------

const opencodeProvider: AgentHistoryProvider = {
  getRecentSessions(scope?: ProjectScope, limit = 20): RecentSession[] {
    const dbPath = getOpenCodeDbPath()

    try {
      if (!fs.existsSync(dbPath)) return []

      const mapRows = (rows: Record<string, unknown>[]): RecentSession[] =>
        rows.map((row) => ({
          sessionId: String(row.id),
          agentType: 'opencode' as AiAgentType,
          display: String(row.title || '').slice(0, 80),
          projectPath: String(row.directory || ''),
          timestamp: Number(row.time_updated) || 0,
          activityCount: Number(row.message_count) || 0,
          activityLabel: getRecentSessionActivityLabel('opencode'),
          canResumeExact: supportsExactSessionResume('opencode')
        }))

      const baseSql =
        'SELECT s.id, s.directory, s.title, s.time_updated, COUNT(m.id) as message_count FROM session s LEFT JOIN message m ON s.id = m.session_id WHERE s.time_archived IS NULL'
      const scopedSql = scope
        ? `${baseSql} AND ${buildPathWhereClause('s.directory', scope)} GROUP BY s.id ORDER BY s.time_updated DESC LIMIT ${limit}`
        : `${baseSql} GROUP BY s.id ORDER BY s.time_updated DESC LIMIT ${limit}`

      let sessions = filterSessionsByProjectPath(mapRows(querySqlite(dbPath, scopedSql)), scope)
      if (scope && sessions.length === 0) {
        const fallbackSql = `${baseSql} GROUP BY s.id ORDER BY s.time_updated DESC`
        sessions = filterSessionsByProjectPath(mapRows(querySqlite(dbPath, fallbackSql)), scope)
      }

      return sessions.slice(0, limit)
    } catch {
      return []
    }
  }
}

// ---------------------------------------------------------------------------
// Registry
// ---------------------------------------------------------------------------

const providerByAgent: Record<AiAgentType, AgentHistoryProvider> = {
  claude: claudeProvider,
  gemini: geminiProvider,
  codex: codexProvider,
  copilot: copilotProvider,
  opencode: opencodeProvider
}

const providers = Object.values(providerByAgent)

export function getRecentSessions(projectPath?: string, limit = 20): RecentSession[] {
  const allSessions: RecentSession[] = []
  const scope = createProjectScope(projectPath)

  for (const provider of providers) {
    allSessions.push(...provider.getRecentSessions(scope, limit))
  }

  return allSessions.sort((a, b) => b.timestamp - a.timestamp).slice(0, limit)
}

/** One agent's sessions, unmerged, so a busy agent cannot crowd out a quiet one. */
export function getRecentSessionsFor(
  agentType: AiAgentType,
  projectPath?: string,
  limit = 20
): RecentSession[] {
  const provider = providerByAgent[agentType]
  if (!provider) return []
  return provider
    .getRecentSessions(createProjectScope(projectPath), limit)
    .sort((a, b) => b.timestamp - a.timestamp)
}
