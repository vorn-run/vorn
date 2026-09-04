import { describe, it, expect } from 'vitest'
import { resolveScriptConfig } from '../src/renderer/lib/workflow-execution'
import type { ScriptConfig } from '../src/shared/types'
import type { StepOutputs } from '../src/renderer/lib/template-vars'

// Where a script runs, resolved the way what it runs already was.

const script = (over: Partial<ScriptConfig> = {}): ScriptConfig => ({
  scriptType: 'bash',
  scriptContent: 'echo hi',
  ...over
})

const steps: StepOutputs = {
  research: { output: 'read the spec', status: 'success', worktreePath: '/tmp/wt/build-1' }
}

describe('the directory a script step runs in', () => {
  it('resolves a run input, so a repo can be chosen when the run starts', () => {
    const resolved = resolveScriptConfig(
      script({ cwd: '{{inputs.repoPath.path}}' }),
      { inputs: { repoPath: { name: 'Novum', path: '/Users/someone/dev/novum' } } },
      {}
    )
    expect(resolved.cwd).toBe('/Users/someone/dev/novum')
  })

  it('resolves an earlier step, so a check runs where the agent worked', () => {
    const resolved = resolveScriptConfig(
      script({ cwd: '{{steps.research.worktreePath}}' }),
      undefined,
      steps
    )
    expect(resolved.cwd).toBe('/tmp/wt/build-1')
  })

  it('resolves the project path the same way', () => {
    const resolved = resolveScriptConfig(
      script({ projectPath: '{{steps.research.worktreePath}}' }),
      undefined,
      steps
    )
    expect(resolved.projectPath).toBe('/tmp/wt/build-1')
  })

  it('leaves the runner its own default when the template names nothing', () => {
    // An empty string would launch in one, rather than falling back to the project.
    const resolved = resolveScriptConfig(
      script({ cwd: '{{steps.missing.worktreePath}}', projectPath: '/plain/path' }),
      undefined,
      steps
    )
    expect(resolved.cwd).toBeUndefined()
    expect(resolved.projectPath).toBe('/plain/path')
  })

  it('says nothing about a step that named no directory', () => {
    const resolved = resolveScriptConfig(script(), undefined, steps)
    expect(resolved.cwd).toBeUndefined()
    expect(resolved.projectPath).toBeUndefined()
  })

  it('still fills in the script itself', () => {
    const resolved = resolveScriptConfig(
      script({ scriptContent: 'cd {{steps.research.worktreePath}} && ls' }),
      undefined,
      steps
    )
    expect(resolved.scriptContent).toBe('cd /tmp/wt/build-1 && ls')
  })

  it('resolves each argument, so untrusted text reaches the script as a value, not as source', () => {
    const resolved = resolveScriptConfig(
      { scriptType: 'bash', scriptContent: 'echo "$1"', args: ['{{inputs.branch}}', 'plain'] },
      { inputs: { branch: 'build/x' } } as never
    )
    expect(resolved.args).toEqual(['build/x', 'plain'])
  })
})
