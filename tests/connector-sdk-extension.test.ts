import { describe, expect, it } from 'vitest'
import {
  connectorManifest,
  defineConnector,
  defineExtension
} from '../packages/connector-sdk/src/index'
import type { ExtensionDefinition, FooterItem } from '../packages/connector-sdk/src/types'

/** The shape the scaffold generates, so a test says which one thing it changed. */
function extension(over: Partial<ExtensionDefinition> = {}) {
  return defineExtension({
    id: 'review',
    name: 'Review',
    description: 'Reads the session',
    version: '0.1.0',
    permissions: ['terminal.read'],
    footers: [
      {
        id: 'checks',
        title: 'Checks',
        description: 'What the last commands said',
        every: 30,
        async run(context): Promise<FooterItem[]> {
          await context.host.output()
          return [{ label: 'tests', value: 'passing', tone: 'ok' }]
        }
      }
    ],
    ...over
  })
}

describe('defineExtension', () => {
  it('keeps what it was given and fills in what a pack needs', () => {
    const built = extension()

    expect(built.kind).toBe('extension')
    expect(built.permissions).toEqual(['terminal.read'])
    expect(built.contributes?.footers?.[0].id).toBe('checks')
    // A connector's own shapes, empty rather than absent, so every seam still reads them.
    expect(built.triggers).toEqual([])
    expect(built.actions).toEqual([])
    expect(built.config).toEqual([])
    // Its credential is the host's own token, so there is nothing to sign in to.
    expect(built.auth).toEqual({ rung: 'none' })
  })

  it('refuses an extension that contributes nothing', () => {
    expect(() => extension({ footers: [] })).toThrow(/contributes nothing/)
  })

  it('refuses a permission this build cannot enforce', () => {
    expect(() => extension({ permissions: ['filesystem.write'] as never })).toThrow(
      /unknown permission/
    )
    expect(() => extension({ permissions: ['git.read', 'git.read'] })).toThrow(
      /Duplicate permission/
    )
  })

  it('refuses two contributions that answer to the same id', () => {
    expect(() =>
      extension({
        panes: [{ id: 'checks', title: 'Checks', web: 'web/checks/index.html' }]
      })
    ).toThrow(/Duplicate contribution "checks"/)
  })

  it('refuses a page that is not one the pack would carry', () => {
    const pane = (web: string) => () =>
      extension({ panes: [{ id: 'report', title: 'Report', web }] })

    expect(pane('../secrets.html')).toThrow(/a page is an \.html file under web\//)
    expect(pane('web/../../etc/passwd.html')).toThrow(/a page is an \.html file under web\//)
    expect(pane('src/report/index.html')).toThrow(/under web\//)
    expect(pane('web/report/index.js')).toThrow(/\.html file/)
    expect(pane('web/report/index.html')).not.toThrow()
  })

  it('refuses a pane that is both a page and a program, or neither', () => {
    expect(() =>
      extension({
        panes: [
          { id: 'report', title: 'Report', web: 'web/r/index.html', command: ['ls'] } as never
        ]
      })
    ).toThrow(/both a web page and a command/)
    expect(() => extension({ panes: [{ id: 'report', title: 'Report' } as never] })).toThrow(
      /neither a web page nor a command/
    )
  })

  it('refuses a command with nothing to run, or an empty argument in it', () => {
    expect(() => extension({ panes: [{ id: 'git', title: 'Git', command: [] }] })).toThrow(
      /nothing to run/
    )
    expect(() =>
      extension({ panes: [{ id: 'git', title: 'Git', command: ['viewer', ''] }] })
    ).toThrow(/empty argument/)
  })

  it('refuses a link handler whose pattern is not a regular expression', () => {
    expect(() =>
      extension({
        linkHandlers: [
          { id: 'pr', title: 'Pull request', pattern: '([', example: 'x', run: () => {} }
        ]
      })
    ).toThrow(/not a regular expression/)
  })

  it('refuses a link handler whose example its own pattern does not match', () => {
    expect(() =>
      extension({
        linkHandlers: [
          {
            id: 'pr',
            title: 'Pull request',
            pattern: 'github\\.com/.+/pull/',
            example: 'https://example.test/nothing',
            run: () => {}
          }
        ]
      })
    ).toThrow(/does not match/)

    expect(() =>
      extension({
        linkHandlers: [
          { id: 'pr', title: 'Pull request', pattern: 'x', example: '  ', run: () => {} }
        ]
      })
    ).toThrow(/names no example link/)
  })

  it('refuses a pattern too long to match on every click', () => {
    expect(() =>
      extension({
        linkHandlers: [
          {
            id: 'pr',
            title: 'Pull request',
            pattern: `${'a'.repeat(257)}`,
            example: 'a',
            run: () => {}
          }
        ]
      })
    ).toThrow(/longer than 256/)
  })

  it('refuses a footer asking to run faster than the host will poll', () => {
    expect(() =>
      extension({
        footers: [{ id: 'spin', title: 'Spin', every: 1, run: () => [] }]
      })
    ).toThrow(/shortest interval/)
  })

  it('refuses a predicate this build cannot evaluate', () => {
    expect(() => extension({ activates: { agent: ['emacs'] as never } })).toThrow(/unknown agent/)
    expect(() => extension({ activates: { platform: ['plan9'] as never } })).toThrow(
      /unknown platform/
    )
    expect(() => extension({ activates: { workspaceContains: [] } })).toThrow(/nothing in it/)
    expect(() => extension({ activates: { workspaceContains: ['../other/Cargo.toml'] } })).toThrow(
      /not inside the worktree/
    )
    expect(() => extension({ activates: { remoteHost: [' '] } })).toThrow(/empty "remoteHost"/)
  })
})

describe("the manifest an extension's pack carries", () => {
  it('says what it contributes, what it asks for, and where it shows — without the code', () => {
    const manifest = connectorManifest(
      extension({
        activates: { workspaceContains: ['package.json'], platform: ['darwin'] },
        panes: [
          { id: 'report', title: 'Report', web: 'web/report/index.html' },
          { id: 'log', title: 'Log', command: ['./bin/log'], when: { agent: ['shell'] } }
        ],
        linkHandlers: [
          {
            id: 'pr',
            title: 'Pull request',
            pattern: 'github\\.com/.+/pull/',
            example: 'https://github.com/vorn-run/vorn/pull/1',
            run: () => {}
          }
        ]
      })
    )

    expect(manifest.kind).toBe('extension')
    expect(manifest.permissions).toEqual(['terminal.read'])
    expect(manifest.activates).toEqual({
      workspaceContains: ['package.json'],
      platform: ['darwin']
    })
    expect(manifest.contributes?.panes).toEqual([
      { id: 'report', title: 'Report', web: 'web/report/index.html' },
      { id: 'log', title: 'Log', command: ['./bin/log'], when: { agent: ['shell'] } }
    ])
    expect(manifest.contributes?.footers).toEqual([
      { id: 'checks', title: 'Checks', description: 'What the last commands said', every: 30 }
    ])
    expect(manifest.contributes?.linkHandlers).toEqual([
      {
        id: 'pr',
        title: 'Pull request',
        pattern: 'github\\.com/.+/pull/',
        example: 'https://github.com/vorn-run/vorn/pull/1'
      }
    ])
    // The code stays in the process that runs it; a manifest is what is written to disk.
    expect(JSON.stringify(manifest)).not.toContain('"run"')
  })

  it('says a connector is a connector, so the two are told apart by name rather than by shape', () => {
    const connector = defineConnector({
      id: 'acme',
      name: 'Acme',
      description: 'Acme tickets',
      triggers: [{ type: 'a', label: 'A', poll: () => ({ items: [] }) }]
    })

    expect(connectorManifest(connector).kind).toBe('connector')
    expect(connectorManifest(connector).contributes).toBeUndefined()
    expect(connectorManifest(connector).permissions).toBeUndefined()
  })
})
