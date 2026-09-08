import { existsSync, readFileSync } from 'node:fs'
import { homedir } from 'node:os'
import { join } from 'node:path'
import type { TerminalSession } from '@vornrun/shared/types'

/**
 * What the session's agent has spent, read from the conversation it is writing.
 *
 * Best effort by design: only some agents publish a usage block, and none of
 * them publish the account allowance. A figure this cannot read is left out
 * rather than guessed, so a footer showing one is showing something true.
 */

export interface ExtensionUsageReading {
  contextTokens?: number
  contextWindow?: number
  cacheHitRate?: number
}

interface TranscriptUsage {
  input_tokens?: number
  output_tokens?: number
  cache_read_input_tokens?: number
  cache_creation_input_tokens?: number
}

/** The directory name a conversation is filed under is its working directory, flattened. */
function transcriptDir(cwd: string): string {
  return cwd.replace(/[^A-Za-z0-9]/g, '-')
}

function transcriptPath(session: TerminalSession, home: string): string | undefined {
  const id = session.agentSessionId
  if (!id) return undefined
  for (const cwd of [session.worktreePath, session.projectPath]) {
    if (!cwd) continue
    const path = join(home, '.claude', 'projects', transcriptDir(cwd), `${id}.jsonl`)
    if (existsSync(path)) return path
  }
  return undefined
}

const isRecord = (value: unknown): value is Record<string, unknown> =>
  typeof value === 'object' && value !== null

/** The usage of the newest assistant turn; earlier ones are already counted in its cache read. */
function lastUsage(contents: string): TranscriptUsage | undefined {
  const lines = contents.split('\n')
  for (let i = lines.length - 1; i >= 0; i--) {
    const line = lines[i].trim()
    if (line === '') continue
    let parsed: unknown
    try {
      parsed = JSON.parse(line)
    } catch {
      continue
    }
    if (!isRecord(parsed) || !isRecord(parsed.message)) continue
    const usage = parsed.message.usage
    if (isRecord(usage)) return usage as TranscriptUsage
  }
  return undefined
}

const count = (value: number | undefined): number => (typeof value === 'number' ? value : 0)

export function usageFor(
  session: TerminalSession,
  home: string = homedir()
): ExtensionUsageReading {
  const path = transcriptPath(session, home)
  if (!path) return {}
  let usage: TranscriptUsage | undefined
  try {
    usage = lastUsage(readFileSync(path, 'utf8'))
  } catch {
    // A conversation being written to, or one this process cannot read, tells us nothing.
    return {}
  }
  if (!usage) return {}

  const cacheRead = count(usage.cache_read_input_tokens)
  const contextTokens =
    count(usage.input_tokens) + count(usage.cache_creation_input_tokens) + cacheRead
  if (contextTokens === 0) return {}
  return { contextTokens, cacheHitRate: cacheRead / contextTokens }
}
