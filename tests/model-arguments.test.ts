import { describe, expect, it, vi } from 'vitest'
vi.mock('node:child_process', () => ({ execFileSync: vi.fn(() => '/bin/agent') }))
import { buildAgentLaunchLine, buildHeadlessSpawnArgs } from '../packages/server/src/agent-launch'
import { applyModelArguments, assertModelCommand } from '../packages/server/src/model-arguments'
import { DEFAULT_AGENT_COMMANDS } from '@vornrun/shared/agent-defaults'
import { validateModelId } from '@vornrun/shared/agent-models'
import type { AiAgentType } from '@vornrun/shared/types'

describe('model overrides', () => {
  it('quotes remote model IDs for the remote POSIX shell even on Windows', () => {
    const platform = vi.spyOn(process, 'platform', 'get').mockReturnValue('win32')
    try {
      const line = buildAgentLaunchLine(
        {
          agentType: 'claude',
          projectName: 'p',
          projectPath: '/p',
          remoteHostId: 'host',
          model: 'opus[1m]'
        },
        DEFAULT_AGENT_COMMANDS,
        {}
      )
      expect(line).toBe("claude --model 'opus[1m]'")
    } finally {
      platform.mockRestore()
    }
  })
  it.each<AiAgentType>(['claude', 'copilot', 'opencode', 'codex'])(
    'preserves advanced args and selects a model for %s',
    (agentType) => {
      const payload = {
        agentType,
        projectName: 'p',
        projectPath: '/p',
        model: 'provider/model',
        args: ['--model=old', '--verbose'],
        initialPrompt: 'hello world'
      }
      const line = buildAgentLaunchLine(payload, DEFAULT_AGENT_COMMANDS, {})
      expect(line).toContain('--model provider/model')
      expect(line).toContain('--verbose')
      expect(line).not.toContain('old')
      const headless = buildHeadlessSpawnArgs(payload, DEFAULT_AGENT_COMMANDS, {})
      expect(headless.args).toContain('provider/model')
      expect(headless.args).toContain('--verbose')
      expect(headless.stdin).toBe('hello world')
    }
  )
  it('removes Codex direct model config overrides but preserves sandbox settings', () => {
    expect(
      applyModelArguments(
        'codex',
        ['-mold', '-c', 'model="old"', '--config=model="other"', '-a', 'never', '-s', 'read-only'],
        'new'
      )
    ).toEqual(['-a', 'never', '-s', 'read-only', '--model', 'new'])
  })
  it('does not interpret positional arguments after the delimiter', () => {
    expect(applyModelArguments('codex', ['--', '--model', 'prompt'], 'new')).toEqual([
      '--model',
      'new',
      '--',
      '--model',
      'prompt'
    ])
  })
  it('rejects unsafe IDs, incomplete flags, and shell wrappers', () => {
    for (const id of ['', '-flag', 'bad\nmodel']) expect(() => validateModelId(id)).toThrow()
    expect(validateModelId(' opus[1m] ')).toBe('opus[1m]')
    expect(() => applyModelArguments('codex', ['--model', '--sandbox'], 'new')).toThrow(
      'incomplete'
    )
    expect(() => assertModelCommand('npx -y codex')).toThrow()
    expect(() => assertModelCommand('/bin/env codex')).toThrow()
    expect(() => assertModelCommand('echo x && codex')).toThrow()
  })
  it('retains legacy defaults and builds exact Codex headless resume', () => {
    const payload = {
      agentType: 'codex' as const,
      projectName: 'p',
      projectPath: '/p',
      resumeSessionId: 'uuid',
      initialPrompt: 'continue',
      model: 'chosen'
    }
    expect(buildHeadlessSpawnArgs(payload, DEFAULT_AGENT_COMMANDS, {})).toEqual({
      command: 'codex',
      args: ['-a', 'never', '--model', 'chosen', 'exec', 'resume', 'uuid', '-'],
      stdin: 'continue'
    })
    expect(buildAgentLaunchLine({ ...payload, model: undefined }, DEFAULT_AGENT_COMMANDS, {})).toBe(
      'codex resume uuid continue'
    )
  })
})
