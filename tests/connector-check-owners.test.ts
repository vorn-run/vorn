import { describe, expect, it } from 'vitest'
import { mkdtempSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import {
  CHECK_OWNERS,
  checkConnector,
  defineConnector,
  packConnector,
  runConformance
} from '../packages/connector-sdk/src/index'
import type { CheckCode } from '../packages/connector-sdk/src/check'

/**
 * Everything a run can say, gathered by actually provoking it.
 *
 * The union keeps `CHECK_OWNERS` complete at compile time; this keeps it
 * honest at runtime, where a cast could still smuggle a code past the type.
 */
async function everyCodeAnyRunEmits(): Promise<Set<string>> {
  const codes = new Set<string>()
  const collect = (findings: Array<{ code: string }>) =>
    findings.forEach((item) => codes.add(item.code))

  // Nothing described, no auth, an action with no outputs and no idempotence.
  collect(
    await checkConnector(
      defineConnector({
        id: 'hollow',
        name: 'Hollow',
        triggers: [{ type: 'quiet', label: 'Quiet', poll: () => ({ items: [] }) }],
        actions: [{ type: 'ping', label: 'Ping', run: () => ({}) }]
      })
    )
  )

  // A credential nobody marked, and a probe the host would drop.
  collect(
    await checkConnector(
      defineConnector({
        id: 'leaky',
        name: 'Leaky',
        description: 'Leaks',
        auth: { rung: 'cli', probe: { command: '/usr/local/bin/glab' } },
        config: [{ key: 'apiToken', label: 'Token' }],
        actions: [
          {
            type: 'go',
            label: 'Go',
            description: 'Go',
            idempotent: true,
            outputs: [{ key: 'ok' }],
            inputs: [{ key: 'level', label: 'Level', type: 'select', description: 'Which' }],
            run: () => ({})
          }
        ]
      })
    )
  )

  // Samples that cannot be replayed, and a live poll that misbehaves.
  const stuck = defineConnector({
    id: 'stuck',
    name: 'Stuck',
    description: 'Stuck',
    auth: { rung: 'none' },
    triggers: [
      {
        type: 'a',
        label: 'A',
        description: 'Never advances',
        poll: () => ({ items: [{ externalId: '1', title: 'One' }], nextCursor: 'same' })
      },
      {
        type: 'b',
        label: 'B',
        description: 'Hand-written with samples',
        poll: () => ({ items: [] }),
        sample: [{ externalId: '1', title: 'One' }]
      }
    ]
  })
  collect(await checkConnector(stuck))
  collect(await checkConnector(stuck, { live: true, config: {} }))

  // A trigger that returns items without a cursor, and one that throws.
  collect(
    await checkConnector(
      defineConnector({
        id: 'cursorless',
        name: 'Cursorless',
        description: 'Cursorless',
        auth: { rung: 'none' },
        triggers: [
          {
            type: 'a',
            label: 'A',
            description: 'No cursor',
            poll: () => ({ items: [{ externalId: '1', title: 'One' }] })
          },
          {
            type: 'b',
            label: 'B',
            description: 'Throws',
            poll: () => {
              throw new Error('boom')
            }
          },
          {
            type: 'c',
            label: 'C',
            description: 'Refuses its own cursor',
            poll: (context) => {
              if (context.cursor) throw new Error('bad cursor')
              return { items: [{ externalId: '1', title: 'One' }], nextCursor: 'next' }
            }
          }
        ]
      })
    )
  )

  // Preflight that refuses, and a live action that fails.
  collect(
    await checkConnector(
      defineConnector({
        id: 'shut',
        name: 'Shut',
        description: 'Shut',
        auth: { rung: 'none' },
        actions: [
          {
            type: 'go',
            label: 'Go',
            description: 'Go',
            idempotent: true,
            outputs: [{ key: 'ok' }],
            run: () => {
              throw new Error('401 Unauthorized')
            }
          }
        ]
      }),
      { live: true, config: {} }
    )
  )
  collect(
    await checkConnector(
      defineConnector({
        id: 'closed',
        name: 'Closed',
        description: 'Closed',
        auth: { rung: 'none' },
        preflight: () => ({ ok: false, message: 'sign in first' }),
        actions: [
          {
            type: 'go',
            label: 'Go',
            description: 'Go',
            idempotent: true,
            outputs: [{ key: 'ok' }],
            run: () => ({})
          }
        ]
      }),
      { live: true, config: {} }
    )
  )

  // A mock run: one action escaping its routes, one the stub never heard.
  const mocked = defineConnector({
    id: 'mocked',
    name: 'Mocked',
    description: 'Mocked',
    auth: { rung: 'none' },
    actions: [
      {
        type: 'away',
        label: 'Away',
        description: 'Reaches out',
        idempotent: true,
        outputs: [{ key: 'ok' }],
        run: async () => {
          await fetch('https://acme.test/elsewhere')
          return {}
        }
      },
      {
        type: 'quiet',
        label: 'Quiet',
        description: 'Asks nothing',
        idempotent: true,
        outputs: [{ key: 'ok' }],
        run: () => ({})
      },
      {
        type: 'angry',
        label: 'Angry',
        description: 'Refuses the reply',
        idempotent: true,
        outputs: [{ key: 'ok' }],
        run: async () => {
          await fetch('https://acme.test/api')
          throw new Error('not what I wanted')
        }
      }
    ]
  })
  collect(await checkConnector(mocked, { mock: true, mockRoutes: [{ url: '/api' }] }))

  // The package gates, and the size gate that only `pack` gives.
  const dir = mkdtempSync(join(tmpdir(), 'vorn-owners-'))
  writeFileSync(join(dir, 'package.json'), JSON.stringify({ scripts: { postinstall: 'x' } }))
  collect(
    await checkConnector(
      defineConnector({
        id: 'shipped',
        name: 'Shipped',
        description: 'Shipped',
        auth: { rung: 'none' },
        actions: [
          {
            type: 'go',
            label: 'Go',
            description: 'Go',
            idempotent: true,
            outputs: [{ key: 'ok' }],
            run: () => ({})
          }
        ]
      }),
      {
        packageDir: dir,
        entry: './index.js',
        bundle: async () => ({ code: '', external: ['left-pad'] })
      }
    )
  )

  const big = await packConnector(
    defineConnector({
      id: 'big',
      name: 'Big',
      version: '1.0.0',
      description: 'Big',
      auth: { rung: 'none' },
      triggers: [{ type: 'a', label: 'A', description: 'A', poll: () => ({ items: [] }) }]
    }),
    {
      entry: './index.js',
      resolveDir: dir,
      outDir: dir,
      maxBytes: 16,
      bundle: async () => ({
        code: `const x = ${JSON.stringify('y'.repeat(4096))}\n`,
        external: []
      })
    }
  )
  collect(big.findings)

  return codes
}

describe('every finding a run can make', () => {
  it('belongs to a named check, or is marked as belonging to none', async () => {
    const emitted = await everyCodeAnyRunEmits()

    // Sanity: the battery really did provoke a broad spread of findings.
    expect(emitted.size).toBeGreaterThan(15)
    for (const code of emitted) {
      expect(Object.keys(CHECK_OWNERS)).toContain(code)
    }
  })

  it('names a check that a run can actually claim, when it names one', async () => {
    const claimable = new Set<string | null>([
      'manifest',
      'auth',
      'secrets',
      'actions',
      'dedupe',
      'no-lifecycle-scripts',
      'keywords',
      'no-runtime-deps',
      'mock',
      'live',
      null
    ])
    for (const owner of Object.values(CHECK_OWNERS)) {
      expect(claimable).toContain(owner)
    }
  })

  it('spoils exactly one check per failing code, so one fault clears one name', async () => {
    const noAuth = defineConnector({
      id: 'acme',
      name: 'Acme',
      description: 'Talks to Acme',
      actions: [
        {
          type: 'go',
          label: 'Go',
          description: 'Go',
          idempotent: true,
          outputs: [{ key: 'ok' }],
          run: () => ({})
        }
      ]
    })
    const run = await runConformance(noAuth)

    // `auth-undeclared` clears `auth` and leaves the rest standing.
    expect(run.findings.map((item) => item.code as CheckCode)).toContain('auth-undeclared')
    expect(run.passed).not.toContain('auth')
    expect(run.passed).toContain('manifest')
    expect(run.passed).toContain('actions')
  })
})
