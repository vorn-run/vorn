import { describe, it, expect, vi, beforeEach } from 'vitest'
import type { SourceConnection } from '../packages/shared/src/types'

vi.mock('../packages/server/src/logger', () => ({
  default: { info: vi.fn(), warn: vi.fn(), error: vi.fn(), debug: vi.fn() }
}))

const listTools = vi.fn()
const callTool = vi.fn()

vi.mock('../packages/server/src/connectors/mcp-clients', () => ({
  getOrStartClient: vi.fn(async () => ({ listTools, callTool })),
  stopClient: vi.fn(),
  stopAllClients: vi.fn()
}))

const { preflightMcpConnection, PREFLIGHT_TOOL } =
  await import('../packages/server/src/connectors/mcp')

const conn = { id: 'c1', connectorId: 'mcp', filters: {} } as unknown as SourceConnection

beforeEach(() => {
  listTools.mockReset()
  callTool.mockReset()
})

describe('preflightMcpConnection', () => {
  /**
   * "Nothing to check" and "checked, fine" are different answers and only one
   * of them should ever be shown to a user as reassurance. Most connectors
   * authenticate from config fields and have nothing to report here.
   */
  it('reports null when the connector registers no preflight tool', async () => {
    listTools.mockResolvedValue({ tools: [{ name: 'poll_issues' }] })
    expect(await preflightMcpConnection(conn)).toEqual({ ok: null })
    expect(callTool).not.toHaveBeenCalled()
  })

  it('reports a passing check', async () => {
    listTools.mockResolvedValue({ tools: [{ name: PREFLIGHT_TOOL }] })
    callTool.mockResolvedValue({ structuredContent: { ok: true } })
    expect(await preflightMcpConnection(conn)).toEqual({ ok: true })
  })

  it('carries the message, which is the part a user can act on', async () => {
    listTools.mockResolvedValue({ tools: [{ name: PREFLIGHT_TOOL }] })
    callTool.mockResolvedValue({
      structuredContent: { ok: false, message: 'Run `gh auth login`.' }
    })
    expect(await preflightMcpConnection(conn)).toEqual({
      ok: false,
      message: 'Run `gh auth login`.'
    })
  })

  // A tool that errors is a failed check, not an absent one — reporting null
  // here would read as "this connector had nothing to verify".
  it('treats a tool error as a failed check', async () => {
    listTools.mockResolvedValue({ tools: [{ name: PREFLIGHT_TOOL }] })
    callTool.mockResolvedValue({
      isError: true,
      content: [{ type: 'text', text: 'connector exploded' }]
    })
    expect(await preflightMcpConnection(conn)).toEqual({
      ok: false,
      message: 'connector exploded'
    })
  })

  it('fails the check when the tool answers with no structured payload', async () => {
    listTools.mockResolvedValue({ tools: [{ name: PREFLIGHT_TOOL }] })
    callTool.mockResolvedValue({ content: [] })
    expect((await preflightMcpConnection(conn)).ok).toBe(false)
  })

  // The fallback reaches a user, so it says something rather than naming an
  // internal tool they never chose.
  it('falls back to a sentence, not the tool name', async () => {
    listTools.mockResolvedValue({ tools: [{ name: PREFLIGHT_TOOL }] })
    callTool.mockResolvedValue({ content: [] })
    const { message } = await preflightMcpConnection(conn)
    expect(message).toBe('The connector could not report whether it is ready.')
    expect(message).not.toContain(PREFLIGHT_TOOL)
  })

  // Anything other than a literal `true` is not a pass. A connector answering
  // `{ok: "yes"}` must not be read as ready.
  it('does not accept a truthy non-boolean as passing', async () => {
    listTools.mockResolvedValue({ tools: [{ name: PREFLIGHT_TOOL }] })
    callTool.mockResolvedValue({ structuredContent: { ok: 'yes' } })
    expect(await preflightMcpConnection(conn)).toEqual({ ok: false })
  })

  it('omits a message that is not a string rather than passing it through', async () => {
    listTools.mockResolvedValue({ tools: [{ name: PREFLIGHT_TOOL }] })
    callTool.mockResolvedValue({ structuredContent: { ok: false, message: { nested: 1 } } })
    expect(await preflightMcpConnection(conn)).toEqual({ ok: false })
  })
})
