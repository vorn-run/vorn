import { describe, expect, it } from 'vitest'
import { mkdirSync, mkdtempSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import {
  checkConnector,
  defineExtension,
  runConformance
} from '../packages/connector-sdk/src/index'
import type { CheckFinding } from '../packages/connector-sdk/src/check'
import type { ExtensionDefinition } from '../packages/connector-sdk/src/types'

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
        async run(context) {
          const output = await context.host.output({ lines: 50 })
          return [{ label: 'tests', value: output.includes('FAIL') ? 'failing' : 'passing' }]
        }
      }
    ],
    ...over
  })
}

const codes = (findings: CheckFinding[]) => findings.map((item) => item.code)

/** A package on disk carrying the page a pane names, so the filesystem check has something to read. */
function packageWithPage(page = 'web/report/index.html'): string {
  const dir = mkdtempSync(join(tmpdir(), 'vorn-extension-'))
  writeFileSync(join(dir, 'package.json'), JSON.stringify({ vorn: { keywords: ['review'] } }))
  mkdirSync(join(dir, 'web', 'report'), { recursive: true })
  writeFileSync(join(dir, page), '<!doctype html>')
  return dir
}

describe('checking an extension', () => {
  it('runs every footer against a stub host and says nothing when it behaves', async () => {
    expect(await checkConnector(extension())).toEqual([])
  })

  it('names a footer that threw', async () => {
    const found = await checkConnector(
      extension({
        footers: [
          {
            id: 'checks',
            title: 'Checks',
            description: 'Throws',
            every: 30,
            run: () => {
              throw new Error('no such file')
            }
          }
        ]
      })
    )

    expect(codes(found)).toContain('footer-failed')
    expect(found[0].message).toContain('no such file')
  })

  it('names a footer returning a reading a band could not draw', async () => {
    const wrong = async (items: unknown) =>
      codes(
        await checkConnector(
          extension({
            footers: [
              {
                id: 'checks',
                title: 'Checks',
                description: 'Wrong shape',
                every: 30,
                run: () => items as never
              }
            ]
          })
        )
      )

    expect(await wrong('passing')).toContain('footer-items-invalid')
    expect(await wrong([{ value: 'passing' }])).toContain('footer-items-invalid')
    expect(await wrong([{ label: 'tests', value: 'ok', tone: 'bright' }])).toContain(
      'footer-items-invalid'
    )
  })

  it('catches a footer reaching for something the extension never declared', async () => {
    const found = await checkConnector(
      extension({
        permissions: ['terminal.read'],
        footers: [
          {
            id: 'checks',
            title: 'Checks',
            description: 'Reaches further than it said',
            every: 30,
            async run(context) {
              await context.host.diff()
              return []
            }
          }
        ]
      })
    )

    expect(codes(found)).toContain('permission-undeclared')
    expect(found[0].message).toContain('git.read')
  })

  it('names a permission that was asked for and never spent', async () => {
    const found = await checkConnector(extension({ permissions: ['terminal.read', 'card.rename'] }))

    expect(codes(found)).toEqual(['permission-unused'])
    expect(found[0].level).toBe('warn')
    expect(found[0].message).toContain('card.rename')
  })

  it('names a link handler that threw', async () => {
    const found = await checkConnector(
      extension({
        linkHandlers: [
          {
            id: 'pr',
            title: 'Pull request',
            pattern: 'github\\.com',
            example: 'https://github.com/vorn-run/vorn/pull/1',
            run: () => {
              throw new Error('nothing to open')
            }
          }
        ]
      })
    )

    expect(codes(found)).toContain('handler-failed')
  })

  it('runs a handler on its own example, so a real pattern is not a failure', async () => {
    const pullRequest = 'https://github\\.com/[^/]+/[^/]+/pull/(\\d+)'
    const found = await checkConnector(
      extension({
        linkHandlers: [
          {
            id: 'pr',
            title: 'Pull request',
            pattern: pullRequest,
            example: 'https://github.com/vorn-run/vorn/pull/42',
            run: (context) => {
              // What a real handler does: read the capture its own pattern declares.
              const matched = new RegExp(pullRequest).exec(context.url)
              if (!matched) throw new Error(`${context.url} is not a pull request`)
              return { openPane: 'report' }
            }
          }
        ]
      })
    )

    expect(codes(found)).toEqual([])
  })

  it('names a reading whose link is not one a card can open', async () => {
    const withHref = (href: string) =>
      extension({
        footers: [
          {
            id: 'checks',
            title: 'Checks',
            description: 'Links somewhere',
            every: 30,
            async run(context) {
              await context.host.output()
              return [{ label: 'ci', value: 'green', href }]
            }
          }
        ]
      })

    expect(codes(await checkConnector(withHref('javascript:alert(1)')))).toContain(
      'footer-items-invalid'
    )
    expect(codes(await checkConnector(withHref('file:///etc/passwd')))).toContain(
      'footer-items-invalid'
    )
    expect(codes(await checkConnector(withHref('not a url')))).toContain('footer-items-invalid')
    expect(codes(await checkConnector(withHref('https://example.test/ci')))).toEqual([])
  })

  it('says nothing about permissions a page spends, since this run cannot watch it', async () => {
    // Declared, never touched by the footer — but the page is where it is spent.
    const withPage = await checkConnector(
      extension({
        permissions: ['terminal.read', 'git.read'],
        panes: [{ id: 'report', title: 'Report', web: 'web/report/index.html' }]
      }),
      { packageDir: packageWithPage() }
    )
    expect(codes(withPage)).toEqual([])

    // The same extension without a page: nothing spends it, and the check says so.
    const withoutPage = await checkConnector(
      extension({ permissions: ['terminal.read', 'git.read'] })
    )
    expect(codes(withoutPage)).toEqual(['permission-unused'])
  })

  it('finds a page from wherever the check was run', async () => {
    const dir = packageWithPage()
    const withPage = extension({
      panes: [{ id: 'report', title: 'Report', web: 'web/report/index.html' }]
    })

    // The entry is under dist/ and the command was run from the parent, which is
    // what `pack` and a monorepo script both do.
    const found = await checkConnector(withPage, {
      packageDir: join(dir, '..'),
      entry: join(dir, 'dist', 'index.js')
    })

    expect(codes(found)).toEqual([])
  })

  it('names a page the package does not carry, and one that resolves outside it', async () => {
    const dir = packageWithPage()

    const missing = await checkConnector(
      extension({ panes: [{ id: 'report', title: 'Report', web: 'web/absent/index.html' }] }),
      { packageDir: dir }
    )
    expect(codes(missing)).toContain('web-entry-missing')

    // Built by hand: `defineExtension` refuses this, so only a pack that skipped it gets here.
    const crafted = {
      ...extension(),
      contributes: {
        panes: [{ id: 'report', title: 'Report', web: '../elsewhere/index.html' }]
      }
    }
    const escaped = await checkConnector(crafted, { packageDir: dir })
    expect(codes(escaped)).toContain('web-entry-outside-package')
  })

  it('says nothing about a page that is there', async () => {
    const found = await checkConnector(
      extension({ panes: [{ id: 'report', title: 'Report', web: 'web/report/index.html' }] }),
      { packageDir: packageWithPage() }
    )

    expect(codes(found)).toEqual([])
  })
})

describe("the receipt an extension's check writes", () => {
  it('vouches for what an extension has, and for nothing a connector would claim', async () => {
    const run = await runConformance(
      extension({
        panes: [{ id: 'report', title: 'Report', web: 'web/report/index.html' }],
        linkHandlers: [
          {
            id: 'pr',
            title: 'Pull request',
            pattern: 'github\\.com',
            example: 'https://github.com/vorn-run/vorn/pull/1',
            run: () => {}
          }
        ]
      }),
      { packageDir: packageWithPage() }
    )

    expect(run.receipt?.checks).toEqual([
      'manifest',
      'contributes',
      'footers',
      'handlers',
      'permissions',
      'no-lifecycle-scripts',
      'keywords'
    ])
    // An extension signs in to nothing, so a receipt saying `auth` would vouch for a question nobody asked.
    expect(run.receipt?.checks).not.toContain('auth')
    expect(run.receipt?.checks).not.toContain('dedupe')
    expect(run.receipt?.checks).not.toContain('actions')
  })

  it('withholds the name a failing contribution belongs to, and keeps the rest', async () => {
    const run = await runConformance(
      extension({
        footers: [
          {
            id: 'checks',
            title: 'Checks',
            description: 'Throws',
            every: 30,
            run: () => {
              throw new Error('boom')
            }
          }
        ]
      })
    )

    expect(run.passed).toContain('manifest')
    expect(run.passed).not.toContain('footers')
    // An error means there is no claim to publish at all.
    expect(run.receipt).toBeUndefined()
  })
})
