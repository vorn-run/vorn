import { describe, it, expect } from 'vitest'
import { parseClientArgs, ClientArgsError } from '../packages/server/src/client-args'

describe('client argument grammar', () => {
  it('keeps the command and its operands in order', () => {
    const args = parseClientArgs(['session', 'logs', 'c3f1a2e8'])
    expect(args.positionals).toEqual(['session', 'logs', 'c3f1a2e8'])
  })

  it('accepts a value attached or separate', () => {
    expect(parseClientArgs(['session', 'logs', 'x', '--lines=200']).lines).toBe(200)
    expect(parseClientArgs(['session', 'logs', 'x', '--lines', '200']).lines).toBe(200)
  })

  it('refuses a count that is not one', () => {
    expect(() => parseClientArgs(['session', 'logs', 'x', '--lines', 'abc'])).toThrow(
      ClientArgsError
    )
    expect(() => parseClientArgs(['session', 'logs', 'x', '--limit', '0'])).toThrow(
      /--limit must be a positive number/
    )
  })

  it('refuses an empty data directory, which would mean the working one', () => {
    expect(() => parseClientArgs(['session', 'list', '--data-dir='])).toThrow(
      /--data-dir needs a directory/
    )
    expect(() => parseClientArgs(['session', 'list', '--data-dir', '  '])).toThrow(ClientArgsError)
  })

  it('reports an unknown option rather than ignoring it', () => {
    expect(() => parseClientArgs(['session', 'list', '--nope'])).toThrow(ClientArgsError)
  })

  it('leaves every flag off until it is given', () => {
    const args = parseClientArgs(['session', 'list'])
    expect(args.json).toBe(false)
    expect(args.headless).toBe(false)
    expect(args.worktree).toBe(false)
    expect(args.recent).toBe(false)
    expect(args.raw).toBe(false)
    expect(args.help).toBe(false)
    expect(args.dataDir).toBeUndefined()
  })
})
