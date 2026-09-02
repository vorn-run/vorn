import { describe, expect, it, vi } from 'vitest'
import { defineConnector, runConformance } from '../packages/connector-sdk/src/index'
import { runCli, type CliDeps } from '../packages/connector-sdk/src/cli'
import type { ConnectorDefinition } from '../packages/connector-sdk/src/types'

const at = () => '2026-09-02T12:00:00.000Z'

/** A connector with nothing for any check to say about it. */
const clean = (over: Partial<ConnectorDefinition> = {}) =>
  defineConnector({
    id: 'acme',
    name: 'Acme',
    version: '1.2.0',
    description: 'Talks to Acme',
    auth: { rung: 'none' },
    actions: [
      {
        type: 'read',
        label: 'Read',
        description: 'Read something back',
        idempotent: true,
        outputs: [{ key: 'ok', type: 'boolean' }],
        run: () => ({ ok: true })
      }
    ],
    ...over
  })

describe('what a conformance run vouches for', () => {
  it('names the checks that ran and had nothing to say', async () => {
    const run = await runConformance(clean(), { now: at })
    expect(run.receipt).toEqual({
      schema: 1,
      version: '1.2.0',
      checkedAt: at(),
      checks: ['manifest', 'auth', 'secrets', 'actions', 'dedupe']
    })
  })

  it('leaves out a check that had something to say, rather than vouching for it', async () => {
    // No auth block is a warning, so the run still passes — but `auth` is not
    // among the things it checked and found clean.
    const run = await runConformance(clean({ auth: undefined }), { now: at })
    expect(run.passed).not.toContain('auth')
    expect(run.receipt?.checks).not.toContain('auth')
    expect(run.receipt).toBeDefined()
  })

  it('makes no claim at all when something failed', async () => {
    const broken = clean({
      actions: [
        {
          type: 'read',
          label: 'Read',
          description: 'Read',
          idempotent: true,
          outputs: [{ key: 'ok' }],
          inputs: [{ key: 'level', label: 'Level', type: 'select', description: 'How much' }],
          run: () => ({})
        }
      ]
    })
    const run = await runConformance(broken, { now: at })
    expect(run.receipt).toBeUndefined()
  })

  it('grows the list with the checks the caller asked for', async () => {
    const run = await runConformance(clean(), { now: at, mock: true })
    expect(run.receipt?.checks).toContain('mock')
  })

  it('does not claim a check nobody ran', async () => {
    const run = await runConformance(clean(), { now: at })
    expect(run.receipt?.checks).not.toContain('mock')
    expect(run.receipt?.checks).not.toContain('live')
    expect(run.receipt?.checks).not.toContain('no-runtime-deps')
  })
})

describe('writing the receipt from the command line', () => {
  const deps = (over: Partial<CliDeps> = {}) => {
    const lines: string[] = []
    const written: Array<{ path: string; contents: string }> = []
    const base: CliDeps = {
      load: async () => ({ default: clean() }),
      write: (line) => lines.push(line),
      writeFile: (path, contents) => written.push({ path, contents }),
      ...over
    }
    return { deps: base, lines, written }
  }

  it('writes what it verified where it was told to', async () => {
    const { deps: cli, written } = deps()
    const code = await runCli(['check', './acme.js', '--receipt', 'verified.json'], cli)

    expect(code).toBe(0)
    expect(written).toHaveLength(1)
    expect(written[0].path).toBe('verified.json')
    const receipt = JSON.parse(written[0].contents) as { schema: number; checks: string[] }
    expect(receipt.schema).toBe(1)
    expect(receipt.checks).toContain('manifest')
  })

  it('writes nothing when it was not asked for a receipt', async () => {
    const { deps: cli, written } = deps()
    await runCli(['check', './acme.js'], cli)
    expect(written).toHaveLength(0)
  })

  it('writes no receipt for a connector that failed, and says so by exit code', async () => {
    const broken = defineConnector({
      id: 'acme',
      name: 'Acme',
      description: 'Talks to Acme',
      auth: { rung: 'none' },
      actions: [
        {
          type: 'read',
          label: 'Read',
          description: 'Read',
          idempotent: true,
          outputs: [{ key: 'ok' }],
          inputs: [{ key: 'level', label: 'Level', type: 'select', description: 'How much' }],
          run: () => ({})
        }
      ]
    })
    const { deps: cli, written } = deps({ load: async () => ({ default: broken }) })

    const code = await runCli(['check', './acme.js', '--receipt', 'verified.json'], cli)
    expect(code).toBe(1)
    expect(written).toHaveLength(0)
  })

  it('runs the package gates too when asked to mock, so one command is the whole gate', async () => {
    const bundle = vi.fn(async () => ({ code: '', external: [] }))
    const { deps: cli } = deps({ bundle, cwd: process.cwd() })

    await runCli(['check', './acme.js', '--mock'], cli)
    expect(bundle).toHaveBeenCalled()
  })
})
