import { describe, it, expect } from 'vitest'
import {
  describeActivation,
  describeContributions,
  describeFooterInterval,
  describePaneKind,
  notAsked,
  permissionRows
} from '../src/renderer/lib/extension-copy'
import type { ExtensionPermission } from '../src/shared/types'

describe('what an extension asks for', () => {
  it('groups a grant under the verb it answers to', () => {
    expect(permissionRows(['terminal.send', 'git.read', 'card.rename'])).toEqual([
      { label: 'Reads', items: ["the worktree's diff and status"] },
      { label: 'Sends', items: ["text into the session's terminal"] },
      { label: 'Renames', items: ['the session card'] }
    ])
  })

  // Two extensions asking for the same things have to read the same, or a
  // reordered manifest looks like a changed request.
  it('reads the same however the manifest ordered it', () => {
    const one: ExtensionPermission[] = ['agent.usage', 'terminal.read', 'git.read']
    const other: ExtensionPermission[] = ['git.read', 'agent.usage', 'terminal.read']
    expect(permissionRows(one)).toEqual(permissionRows(other))
    expect(permissionRows(one)[0].items).toEqual([
      "the worktree's diff and status",
      "the session's recent terminal output",
      "the agent's context and provider allowance"
    ])
  })

  it('says nothing for a bucket it asked nothing of', () => {
    expect(permissionRows(['git.read']).map((row) => row.label)).toEqual(['Reads'])
    expect(permissionRows([])).toEqual([])
    expect(permissionRows(undefined)).toEqual([])
  })

  // A grant reads as small only against the list it was drawn from.
  it('names what it did not ask for, from the same closed list', () => {
    expect(notAsked(['git.read', 'terminal.read'])).toEqual([
      'the text selected in the terminal',
      "the agent's context and provider allowance",
      "text into the session's terminal",
      'the session card'
    ])
    expect(notAsked(undefined)).toHaveLength(6)
    expect(
      notAsked([
        'git.read',
        'terminal.read',
        'terminal.selection',
        'terminal.send',
        'card.rename',
        'agent.usage'
      ])
    ).toEqual([])
  })
})

describe('where an extension shows', () => {
  it('says every session when it names no rule', () => {
    expect(describeActivation(undefined)).toEqual(['every session'])
    expect(describeActivation({})).toEqual(['every session'])
  })

  it('reads a path as the projects that have it', () => {
    expect(describeActivation({ workspaceContains: ['package.json'] })).toEqual([
      'projects with package.json'
    ])
  })

  // Any one value satisfies a field, so the clause is an "or" inside.
  it('joins the values of one field with or', () => {
    expect(describeActivation({ workspaceContains: ['package.json', 'Cargo.toml'] })).toEqual([
      'projects with package.json or Cargo.toml'
    ])
    expect(describeActivation({ platform: ['darwin', 'linux', 'win32'] })).toEqual([
      'macOS, Linux or Windows'
    ])
  })

  it('names a host it knows and says the rest as they are', () => {
    expect(describeActivation({ remoteHost: ['github.com'] })).toEqual(['GitHub remotes'])
    expect(describeActivation({ remoteHost: ['git.example.com'] })).toEqual([
      'git.example.com remotes'
    ])
  })

  // Any one of a field's values is enough, so two of them are one clause. Two
  // clauses would read as both being required, which is the opposite.
  it('reads a list within one field as any of them', () => {
    expect(describeActivation({ remoteHost: ['github.com', 'git.example.com'] })).toEqual([
      'GitHub or git.example.com remotes'
    ])
  })

  it('names an agent the way the rest of the app names it', () => {
    expect(describeActivation({ agent: ['claude'] })).toEqual(['Claude Code sessions'])
    expect(describeActivation({ agent: ['shell'] })).toEqual(['shell sessions'])
  })

  // Every declared field has to hold, so the clauses read as a list of conditions.
  it('keeps one clause per field it declares', () => {
    expect(
      describeActivation({
        workspaceContains: ['package.json'],
        remoteHost: ['github.com'],
        agent: ['claude'],
        platform: ['darwin']
      })
    ).toEqual(['projects with package.json', 'GitHub remotes', 'Claude Code sessions', 'macOS'])
  })
})

describe('what an extension adds', () => {
  it('counts each kind, and leaves out a kind it has none of', () => {
    expect(
      describeContributions({
        panes: [
          { id: 'a', title: 'A' },
          { id: 'b', title: 'B' }
        ],
        footers: [{ id: 'c', title: 'C', every: 30 }]
      })
    ).toBe('2 panes, 1 footer')
    expect(describeContributions({ linkHandlers: [{ id: 'd', title: 'D', pattern: 'x' }] })).toBe(
      '1 link handler'
    )
    expect(describeContributions(undefined)).toBe('')
  })

  it('says how a pane is drawn', () => {
    expect(describePaneKind({ web: 'web/report/index.html' })).toBe('a page it ships')
    expect(describePaneKind({ command: ['top'] })).toBe('runs top')
  })

  // Claiming a page for a pane that ships none would be describing a file that is not there.
  it('says nothing about a pane that names neither a page nor a program', () => {
    expect(describePaneKind({})).toBe('a pane')
  })

  it('says an interval the way a person would', () => {
    expect(describeFooterInterval(30)).toBe('every 30s')
    expect(describeFooterInterval(60)).toBe('every minute')
    expect(describeFooterInterval(300)).toBe('every 5 minutes')
  })
})
