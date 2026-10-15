/**
 * The SDK writes `vorn-extension.toml` for a connector's folder; vornd reads
 * it strictly. The manifests written here for every shape a connector or a
 * card extension can take are kept as fixtures, and
 * `packages/core/crates/extensions/tests/manifests.rs` reads each one, so a
 * key the SDK writes that Vorn would refuse fails one side or the other.
 *
 * VORN_UPDATE_FIXTURES=1 writes the fixtures again.
 */
import { readFileSync, writeFileSync } from 'node:fs'
import { join, resolve } from 'node:path'
import { describe, expect, it } from 'vitest'
import { CHECK_OWNERS } from '../packages/connector-sdk/src/check'
import { extensionToml, RECORDED_CHECKS, selfCheck } from '../packages/connector-sdk/src/extension'
import type { ConnectorManifest } from '../packages/connector-sdk/src/setup'

const FIXTURES = resolve(
  __dirname,
  '../packages/core/crates/extensions/tests/fixtures/sdk-manifests'
)

const setup = (id: string, type: string): ConnectorManifest['triggers'][number]['setup'] => ({
  connectorId: id,
  triggerType: type,
  env: []
})

const browserConnector: ConnectorManifest = {
  protocol: 1,
  id: 'sample',
  name: 'Sample "quoted"',
  version: '0.2.0',
  kind: 'connector',
  description: 'Reads a site you sign in to\nand acts on it — both ways.',
  icon: { viewBox: '0 0 24 24', paths: ['M0 0h24v24H0z'] },
  auth: {
    rung: 'browser',
    browser: {
      signInUrl: 'https://sample.example/sign-in',
      origins: ['https://sample.example', 'https://*.sample.example'],
      check: {
        url: 'https://sample.example/api/me',
        identity: ['name'],
        headers: { 'X-Requested-With': 'vorn' }
      }
    }
  },
  triggers: [
    {
      type: 'post',
      label: 'New post',
      description: 'A post appears.',
      statusMapping: [{ upstream: 'open', suggestedLocal: 'todo' }],
      defaultWorkflow: { name: 'Posts to tasks', defaultCronFromMinutes: 15 },
      setup: setup('sample', 'post')
    },
    { type: 'same', label: 'same', setup: setup('sample', 'same') }
  ],
  actions: [
    {
      type: 'comment',
      label: 'Comment',
      description: 'Leaves a comment.',
      idempotent: false,
      inputs: [
        { key: 'body', label: 'Body', type: 'string', required: true, builderHint: 'dropped' },
        {
          key: 'kind',
          label: 'Kind',
          type: 'select',
          required: false,
          options: [{ value: 'a', label: 'A' }, { value: 'b' }]
        },
        { key: 'board', label: 'Board', type: 'select', required: false, loadOptions: 'boards' }
      ],
      outputs: [{ key: 'id', type: 'string', description: 'The comment' }, { key: 'url' }]
    }
  ]
}

const cliConnector: ConnectorManifest = {
  protocol: 1,
  id: 'cli-sample',
  name: 'CLI sample',
  version: '1.0.0',
  kind: 'connector',
  auth: {
    rung: 'cli',
    probe: { command: 'gh', args: ['auth', 'status'] },
    borrow: { env: ['GH_TOKEN'], tokenArgs: ['auth', 'token'], tokenEnv: 'GH_TOKEN' }
  },
  triggers: [{ type: 'issue', label: 'Issue', setup: setup('cli-sample', 'issue') }],
  actions: []
}

const cardExtension: ConnectorManifest = {
  protocol: 1,
  id: 'card-sample',
  name: 'Card sample',
  version: '0.1.0',
  kind: 'extension',
  permissions: ['git.read', 'terminal.read'],
  activates: { agent: ['claude'] },
  contributes: {
    panes: [
      {
        id: 'report',
        title: 'Report',
        web: 'web/report/index.html',
        when: { workspaceContains: ['package.json'] },
        icon: { viewBox: '0 0 24 24', paths: ['M1 1h2'] }
      }
    ],
    footers: [{ id: 'status', title: 'Status', every: 5 }],
    linkHandlers: [
      { id: 'issue', title: 'Open issue', pattern: '\\b[A-Z]+-\\d+\\b', example: 'ABC-1' }
    ]
  },
  triggers: [],
  actions: []
}

const cases: Array<[string, ConnectorManifest, Parameters<typeof extensionToml>[1]]> = [
  [
    'browser',
    browserConnector,
    { hosts: ['sample.example', '*.sample.example'], publisher: 'Tests' }
  ],
  ['cli', cliConnector, { hosts: ['api.github.com'], hostsFrom: ['GH_HOSTS'] }],
  ['card', cardExtension, {}]
]

describe('the vorn-extension.toml the SDK writes', () => {
  for (const [name, manifest, extras] of cases) {
    it(`writes ${name} as the fixture Vorn's reader is held to`, () => {
      const file = join(FIXTURES, `${name}.toml`)
      const written = extensionToml(manifest, extras)
      if (process.env.VORN_UPDATE_FIXTURES === '1') writeFileSync(file, written)
      expect(written).toBe(readFileSync(file, 'utf8'))
    })
  }

  it('drops what Vorn does not read and says a label only when it differs', () => {
    const written = extensionToml(browserConnector)
    expect(written).not.toContain('builderHint')
    expect(written).not.toContain('idempotent')
    expect(written).toContain('[[connector.triggers]]\ntype = "same"\n')
    expect(written).toContain('name = "Sample \\"quoted\\""')
  })

  it('names every check a receipt can record', () => {
    const owners = new Set(Object.values(CHECK_OWNERS).filter((o): o is string => o !== null))
    expect([...RECORDED_CHECKS].sort()).toEqual([...owners].sort())
  })

  it('holds a bundle to the receipt beside it', () => {
    const m = { id: 'x', version: '1.0.0' }
    expect(
      selfCheck(m, { version: '1.0.0', checks: ['manifest', 'launch'] }, RECORDED_CHECKS)
    ).toEqual([])
    expect(selfCheck(m, undefined, RECORDED_CHECKS)).toEqual([
      'verified.json is missing or is not an object'
    ])
    expect(selfCheck(m, { version: '0.9.0', checks: ['nope'] }, RECORDED_CHECKS)).toEqual([
      'verified.json records 0.9.0, and the bundle is 1.0.0',
      'verified.json records an unknown check nope'
    ])
  })
})
