import { describe, it, expect } from 'vitest'
import { parseServerArgs, ServerArgsError } from '../packages/server/src/server-args'

/**
 * One grammar for both entry points — the process Electron forks and the
 * `vorn-server` CLI. They used to parse separately and had already drifted: only
 * one accepted `--port 3000`, and only one rejected a non-numeric port.
 */
describe('parseServerArgs', () => {
  it('returns nothing set for an empty command line', () => {
    expect(parseServerArgs([])).toEqual({
      host: undefined,
      port: undefined,
      dataDir: undefined,
      name: undefined,
      help: false,
      positionals: []
    })
  })

  it.each([
    ['inline', ['--port=4400']],
    ['spaced', ['--port', '4400']]
  ])('accepts a %s port', (_label, argv) => {
    expect(parseServerArgs(argv).port).toBe(4400)
  })

  it.each([
    ['inline', ['--data-dir=/srv/vorn']],
    ['spaced', ['--data-dir', '/srv/vorn']]
  ])('accepts a %s data dir', (_label, argv) => {
    expect(parseServerArgs(argv).dataDir).toBe('/srv/vorn')
  })

  it('rejects a non-numeric port instead of passing NaN to listen()', () => {
    expect(() => parseServerArgs(['--port=abc'])).toThrow(ServerArgsError)
    expect(() => parseServerArgs(['--port=abc'])).toThrow(/must be a number/)
  })

  it('reports an unknown option rather than ignoring it', () => {
    expect(() => parseServerArgs(['--nope'])).toThrow(ServerArgsError)
  })

  it('keeps positionals in order and apart from flag values', () => {
    const args = parseServerArgs(['token', 'revoke', 'abc-123', '--data-dir', '/srv/vorn'])
    expect(args.positionals).toEqual(['token', 'revoke', 'abc-123'])
    expect(args.dataDir).toBe('/srv/vorn')
  })

  it.each([['--help'], ['-h']])('recognises %s', (flag) => {
    expect(parseServerArgs([flag]).help).toBe(true)
  })

  it('parses the arguments the desktop launcher and CLI actually send', () => {
    // The launcher passes no arguments at all, so the data dir resolves to the
    // default — see the note in server-launcher.ts for why it must not pass one.
    expect(parseServerArgs([]).dataDir).toBeUndefined()

    const serve = parseServerArgs(['serve', '--port=4400', '--data-dir=/srv/vorn'])
    expect(serve.positionals).toEqual(['serve'])
    expect(serve.port).toBe(4400)
    expect(serve.dataDir).toBe('/srv/vorn')

    const create = parseServerArgs(['token', 'create', '--name', "Javier's iPhone"])
    expect(create.positionals).toEqual(['token', 'create'])
    expect(create.name).toBe("Javier's iPhone")
  })
})
