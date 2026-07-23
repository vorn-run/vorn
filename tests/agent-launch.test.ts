import { describe, it, expect, vi, beforeEach } from 'vitest'

vi.mock('node:child_process', () => ({
  execFileSync: vi.fn(() => '/usr/bin/cmd') // commandExists returns true
}))

import {
  buildAgentLaunchLine,
  buildHeadlessLaunchLine,
  buildHeadlessSpawnArgs
} from '../packages/server/src/agent-launch'
import { DEFAULT_AGENT_COMMANDS } from '@vornrun/shared/agent-defaults'
import type { AgentType, CreateTerminalPayload } from '@vornrun/shared/types'

const env = { PATH: '/usr/bin' }
const cmds = DEFAULT_AGENT_COMMANDS

function makePayload(overrides: Partial<CreateTerminalPayload> = {}): CreateTerminalPayload {
  return {
    agentType: 'claude' as AgentType,
    projectName: 'test',
    projectPath: '/test',
    ...overrides
  }
}

beforeEach(() => {
  vi.clearAllMocks()
})

describe('buildAgentLaunchLine', () => {
  it('returns basic claude command', () => {
    expect(buildAgentLaunchLine(makePayload(), cmds, env)).toBe('claude')
  })

  it('adds --resume for claude', () => {
    const result = buildAgentLaunchLine(makePayload({ resumeSessionId: 'sess-1' }), cmds, env)
    expect(result).toBe('claude --resume sess-1')
  })

  it('adds prompt for claude (appended directly)', () => {
    const result = buildAgentLaunchLine(makePayload({ initialPrompt: 'fix bug' }), cmds, env)
    expect(result).toContain("'fix bug'")
  })

  it('uses -i flag for copilot initialPrompt', () => {
    const result = buildAgentLaunchLine(
      makePayload({ agentType: 'copilot', initialPrompt: 'fix' }),
      cmds,
      env
    )
    expect(result).toContain('-i')
  })

  it('uses resume subcommand for codex', () => {
    const result = buildAgentLaunchLine(
      makePayload({ agentType: 'codex', resumeSessionId: 'sess-1' }),
      cmds,
      env
    )
    expect(result).toBe('codex resume sess-1')
  })

  it('uses --session for opencode', () => {
    const result = buildAgentLaunchLine(
      makePayload({ agentType: 'opencode', resumeSessionId: 'sess-1' }),
      cmds,
      env
    )
    expect(result).toContain('--session sess-1')
  })

  it('does not inject a fake exact-resume flag for gemini', () => {
    const result = buildAgentLaunchLine(
      makePayload({ agentType: 'gemini', resumeSessionId: 'any-id' }),
      cmds,
      env
    )
    expect(result).toBe('gemini')
  })

  it('uses per-step args over settings-level args', () => {
    const result = buildAgentLaunchLine(makePayload({ args: ['--verbose'] }), cmds, env)
    expect(result).toContain('--verbose')
  })

  it('adds --session-id for fresh Claude session', () => {
    const result = buildAgentLaunchLine(makePayload({ sessionId: 'uuid-123' }), cmds, env)
    expect(result).toBe('claude --session-id uuid-123')
  })

  it('does not add --session-id when resumeSessionId is present', () => {
    const result = buildAgentLaunchLine(
      makePayload({ resumeSessionId: 'sess-1', sessionId: 'uuid-123' }),
      cmds,
      env
    )
    expect(result).toBe('claude --resume sess-1')
    expect(result).not.toContain('--session-id')
  })

  it('pins fresh copilot session via --session-id', () => {
    const result = buildAgentLaunchLine(
      makePayload({ agentType: 'copilot', sessionId: 'uuid-123' }),
      cmds,
      env
    )
    expect(result).toBe('copilot --session-id uuid-123')
    expect(result).not.toContain('--resume')
  })

  it('prefers resumeSessionId over pinned sessionId for copilot', () => {
    const result = buildAgentLaunchLine(
      makePayload({ agentType: 'copilot', resumeSessionId: 'sess-1', sessionId: 'uuid-123' }),
      cmds,
      env
    )
    expect(result).toBe('copilot --resume sess-1')
  })

  it('does not add --session-id for non-pinning agents', () => {
    const result = buildAgentLaunchLine(
      makePayload({ agentType: 'codex', sessionId: 'uuid-123' }),
      cmds,
      env
    )
    expect(result).not.toContain('--session-id')
    expect(result).not.toContain('--resume')
  })
})

describe('buildHeadlessLaunchLine', () => {
  it('builds claude with -p and headlessArgs', () => {
    const result = buildHeadlessLaunchLine(makePayload({ initialPrompt: 'do it' }), cmds, env)
    expect(result).toContain('claude')
    expect(result).toContain('--dangerously-skip-permissions')
    expect(result).toContain('-p')
  })

  it('builds copilot with --allow-all', () => {
    const result = buildHeadlessLaunchLine(
      makePayload({ agentType: 'copilot', initialPrompt: 'do it' }),
      cmds,
      env
    )
    expect(result).toContain('--allow-all')
    expect(result).toContain('-p')
  })

  it('builds codex with exec subcommand', () => {
    const result = buildHeadlessLaunchLine(
      makePayload({ agentType: 'codex', initialPrompt: 'do it' }),
      cmds,
      env
    )
    expect(result).toContain('exec')
    expect(result).toContain('-a never')
  })

  it('builds opencode with run subcommand', () => {
    const result = buildHeadlessLaunchLine(
      makePayload({ agentType: 'opencode', initialPrompt: 'do it' }),
      cmds,
      env
    )
    expect(result).toContain('run')
  })

  it('builds gemini with -y flag', () => {
    const result = buildHeadlessLaunchLine(
      makePayload({ agentType: 'gemini', initialPrompt: 'do it' }),
      cmds,
      env
    )
    expect(result).toContain('-y')
    expect(result).toContain('-p')
  })

  it('uses empty quoted string when no prompt', () => {
    const result = buildHeadlessLaunchLine(makePayload(), cmds, env)
    expect(result).toContain("''")
  })

  it('per-step args override headlessArgs', () => {
    const result = buildHeadlessLaunchLine(makePayload({ args: ['--custom'] }), cmds, env)
    expect(result).toContain('--custom')
    expect(result).not.toContain('--dangerously-skip-permissions')
  })
})

describe('buildHeadlessSpawnArgs', () => {
  it('returns { command, args } for claude with the prompt on stdin', () => {
    const result = buildHeadlessSpawnArgs(makePayload({ initialPrompt: 'hello' }), cmds, env)
    expect(result.command).toBe('claude')
    expect(result.args).toContain('-p')
    expect(result.args).toContain('--dangerously-skip-permissions')
    // Prompt goes to stdin, not argv, so the Windows shell command line can't
    // word-split or line-break it.
    expect(result.stdin).toBe('hello')
    expect(result.args).not.toContain('hello')
  })

  it('keeps a multi-line claude prompt intact on stdin', () => {
    const prompt = '# Workflow: Demo\n\n**Step:** one\n\nDo the thing.'
    const result = buildHeadlessSpawnArgs(makePayload({ initialPrompt: prompt }), cmds, env)
    expect(result.stdin).toBe(prompt)
    // `-p` is a bare flag here; the last argv element must not be the prompt.
    expect(result.args[result.args.length - 1]).toBe('-p')
  })

  it('returns exec for codex', () => {
    const result = buildHeadlessSpawnArgs(
      makePayload({ agentType: 'codex', initialPrompt: 'fix' }),
      cmds,
      env
    )
    expect(result.args).toContain('exec')
    expect(result.args).toContain('fix')
  })

  it('returns run for opencode', () => {
    const result = buildHeadlessSpawnArgs(
      makePayload({ agentType: 'opencode', initialPrompt: 'fix' }),
      cmds,
      env
    )
    expect(result.args).toContain('run')
  })

  it('uses empty string for missing prompt', () => {
    const result = buildHeadlessSpawnArgs(makePayload(), cmds, env)
    expect(result.args).toContain('')
    expect(result.stdin).toBeUndefined()
  })

  it('pins claude headless session via --session-id', () => {
    const result = buildHeadlessSpawnArgs(
      makePayload({ sessionId: 'uuid-head', initialPrompt: 'go' }),
      cmds,
      env
    )
    const idx = result.args.indexOf('--session-id')
    expect(idx).toBeGreaterThanOrEqual(0)
    expect(result.args[idx + 1]).toBe('uuid-head')
  })

  it('pins copilot headless session via --session-id', () => {
    const result = buildHeadlessSpawnArgs(
      makePayload({ agentType: 'copilot', sessionId: 'uuid-head', initialPrompt: 'go' }),
      cmds,
      env
    )
    const idx = result.args.indexOf('--session-id')
    expect(idx).toBeGreaterThanOrEqual(0)
    expect(result.args[idx + 1]).toBe('uuid-head')
    expect(result.args).not.toContain('--resume')
  })

  it('resumes claude headless via --resume', () => {
    const result = buildHeadlessSpawnArgs(
      makePayload({ resumeSessionId: 'sess-prev', initialPrompt: 'go' }),
      cmds,
      env
    )
    const idx = result.args.indexOf('--resume')
    expect(idx).toBeGreaterThanOrEqual(0)
    expect(result.args[idx + 1]).toBe('sess-prev')
  })

  it('resume wins over session-id pinning', () => {
    const result = buildHeadlessSpawnArgs(
      makePayload({ sessionId: 'uuid-head', resumeSessionId: 'sess-prev', initialPrompt: 'go' }),
      cmds,
      env
    )
    expect(result.args).toContain('--resume')
    expect(result.args).toContain('sess-prev')
    expect(result.args).not.toContain('--session-id')
  })

  it('does not inject session-id or resume flags for codex/opencode/gemini headless', () => {
    for (const agentType of ['codex', 'opencode', 'gemini'] as const) {
      const result = buildHeadlessSpawnArgs(
        makePayload({ agentType, sessionId: 'uuid-head', resumeSessionId: 'sess-prev' }),
        cmds,
        env
      )
      expect(result.args).not.toContain('--session-id')
      expect(result.args).not.toContain('--resume')
    }
  })
})

// A workflow prompt (buildWorkflowPrompt) is multi-word and multi-line. On
// Windows the headless spawn runs through `shell: true`, which word-splits
// unquoted argv — the trigger for issue #374 (claude got only `#`) and the
// equivalent copilot "extra words were treated as separate arguments" failure.
// Every supported agent must therefore deliver the ENTIRE prompt as one unit:
// claude on stdin (bare `-p`), everyone else as a single argv element that the
// spawn site can quote. These tests lock that contract in per agent.
describe('buildHeadlessSpawnArgs — every agent delivers the whole prompt as one unit', () => {
  const PROMPT = '# Workflow: Demo\n\n**Step:** one\n\nDo the thing with spaces.'

  it('claude delivers the prompt on stdin, never on argv', () => {
    const result = buildHeadlessSpawnArgs(
      makePayload({ agentType: 'claude', initialPrompt: PROMPT }),
      cmds,
      env
    )
    expect(result.command).toBe('claude')
    expect(result.stdin).toBe(PROMPT)
    expect(result.args).not.toContain(PROMPT)
    // `-p` present with no positional prompt following it.
    expect(result.args).toContain('-p')
    expect(result.args[result.args.length - 1]).toBe('-p')
  })

  // For the arg-based agents the prompt must be exactly one array element so
  // it can be quoted as a single token — never spread across several.
  it.each([
    { agentType: 'copilot' as const, command: 'copilot', flag: '--allow-all' },
    { agentType: 'codex' as const, command: 'codex', flag: 'exec' },
    { agentType: 'opencode' as const, command: 'opencode', flag: 'run' },
    { agentType: 'gemini' as const, command: 'gemini', flag: '-y' }
  ])('$agentType delivers the prompt as a single argv element', ({ agentType, command, flag }) => {
    const result = buildHeadlessSpawnArgs(
      makePayload({ agentType, initialPrompt: PROMPT }),
      cmds,
      env
    )
    expect(result.command).toBe(command)
    expect(result.stdin).toBeUndefined()
    expect(result.args).toContain(flag)
    // The full multi-word/multi-line prompt is a single element, not split.
    expect(result.args).toContain(PROMPT)
    expect(result.args.filter((a) => a === PROMPT)).toHaveLength(1)
  })
})

describe('agent-launch guards against shell sessions', () => {
  // Shells don't go through this file — they have their own PTY creation path.
  // Guards exist so that if something mistakenly routes a shell through here,
  // we surface the bug instead of silently running the wrong command.
  const shellPayload = makePayload({ agentType: 'shell' as AgentType })

  it('buildAgentLaunchLine throws for shell payloads', () => {
    expect(() => buildAgentLaunchLine(shellPayload, cmds, env)).toThrow(
      /buildAgentLaunchLine called for shell session/
    )
  })

  it('buildHeadlessLaunchLine throws for shell payloads', () => {
    expect(() => buildHeadlessLaunchLine(shellPayload, cmds, env)).toThrow(
      /buildHeadlessLaunchLine called for shell session/
    )
  })

  it('buildHeadlessSpawnArgs throws for shell payloads', () => {
    expect(() => buildHeadlessSpawnArgs(shellPayload, cmds, env)).toThrow(
      /buildHeadlessSpawnArgs called for shell session/
    )
  })
})
