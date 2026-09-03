import { describe, it, expect } from 'vitest'
import { withProjectInputs } from '../src/renderer/lib/workflow-execution'
import { resolveTemplateVars } from '../src/renderer/lib/template-vars'
import type { ProjectConfig, WorkflowInputDef } from '../src/shared/types'

/**
 * A project input, and the directory it stands for.
 *
 * The run dialog stores the name someone picked; a step that has to run there
 * needs the path, and nothing in a template could reach it.
 */

const projects: ProjectConfig[] = [
  { name: 'Novum', path: '/Users/someone/dev/novum', preferredAgents: [] },
  { name: 'Other', path: '/Users/someone/dev/other', preferredAgents: [] }
]

const defs: WorkflowInputDef[] = [
  { key: 'repo', label: 'Repository', type: 'project' },
  { key: 'branch', label: 'Branch', type: 'text' }
]

describe('a project input on the way into a run', () => {
  it('carries the directory beside the name', () => {
    const inputs = withProjectInputs({ repo: 'Novum', branch: 'build/x' }, defs, projects)
    expect(inputs).toEqual({
      repo: { name: 'Novum', path: '/Users/someone/dev/novum' },
      branch: 'build/x'
    })
  })

  it('leaves every other kind of input exactly as it was', () => {
    const inputs = { branch: 'build/x', count: 3 }
    expect(withProjectInputs(inputs, defs, projects)).toBe(inputs)
  })

  it('leaves a name this machine does not have alone, rather than inventing a path', () => {
    expect(withProjectInputs({ repo: 'Gone' }, defs, projects)).toEqual({ repo: 'Gone' })
  })

  it('passes an enriched value through, so a re-run does not wrap it twice', () => {
    const already = { repo: { name: 'Novum', path: '/Users/someone/dev/novum' } }
    expect(withProjectInputs(already, defs, projects)).toBe(already)
  })

  it('says nothing about a run that supplied no inputs', () => {
    expect(withProjectInputs(undefined, defs, projects)).toBeUndefined()
  })
})

describe('what a template makes of one', () => {
  const context = {
    inputs: withProjectInputs({ repo: 'Novum', branch: 'build/x' }, defs, projects)
  }

  it('answers the bare name, which is what was picked', () => {
    expect(resolveTemplateVars('{{inputs.repo}}', context)).toBe('Novum')
  })

  it('answers the directory when asked for it', () => {
    expect(resolveTemplateVars('{{inputs.repo.path}}', context)).toBe('/Users/someone/dev/novum')
    expect(resolveTemplateVars('{{inputs.repo.name}}', context)).toBe('Novum')
  })

  it('leaves a plain input reading as itself', () => {
    expect(resolveTemplateVars('{{inputs.branch}}', context)).toBe('build/x')
  })
})
