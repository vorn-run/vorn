import { describe, expect, it, vi } from 'vitest'
import {
  createExtensionHost,
  HostReplyError,
  HOST_TOKEN_ENV,
  HOST_URL_ENV,
  PermissionDeniedError
} from '../packages/connector-sdk/src/host'

/**
 * The bridge an extension process reaches the host through.
 *
 * Two things are being pinned. That the token goes only to this machine — the
 * variable naming the bridge is read from the environment, and an address
 * pointing elsewhere would hand the grant away — and that an answer the bridge
 * cannot read fails by name rather than as a TypeError from inside JSON.parse.
 */
function hostWith(
  reply: { status?: number; body: string },
  env: NodeJS.ProcessEnv = {
    [HOST_URL_ENV]: 'http://127.0.0.1:8931/bridge',
    [HOST_TOKEN_ENV]: 'secret'
  }
) {
  const fetchImpl = vi.fn(async () =>
    Promise.resolve({
      ok: (reply.status ?? 200) < 400,
      status: reply.status ?? 200,
      text: async () => Promise.resolve(reply.body)
    })
  ) as unknown as typeof fetch

  return { host: createExtensionHost({ sessionId: 's1', env, fetchImpl }), fetchImpl }
}

describe('the bridge an extension asks the host through', () => {
  it('sends the session and the token to the address Vorn named', async () => {
    const { host, fetchImpl } = hostWith({ body: JSON.stringify({ result: 'M src/index.ts' }) })

    expect(await host.status()).toBe('M src/index.ts')
    const [url, init] = vi.mocked(fetchImpl).mock.calls[0] as [string, RequestInit]
    expect(url).toBe('http://127.0.0.1:8931/bridge/status')
    expect((init.headers as Record<string, string>).authorization).toBe('Bearer secret')
    expect(JSON.parse(String(init.body))).toEqual({ sessionId: 's1' })
  })

  it('refuses to send the token anywhere but this machine', async () => {
    for (const url of [
      'https://example.test/bridge',
      'http://example.test/bridge',
      'http://127.0.0.1.example.test/bridge',
      'not a url'
    ]) {
      const { host, fetchImpl } = hostWith(
        { body: '{}' },
        { [HOST_URL_ENV]: url, [HOST_TOKEN_ENV]: 'secret' }
      )
      await expect(host.status()).rejects.toThrow(new RegExp(HOST_URL_ENV))
      expect(fetchImpl).not.toHaveBeenCalled()
    }

    for (const url of ['http://localhost:8931', 'http://[::1]:8931']) {
      const { host } = hostWith(
        { body: JSON.stringify({ result: '' }) },
        { [HOST_URL_ENV]: url, [HOST_TOKEN_ENV]: 'secret' }
      )
      await expect(host.status()).resolves.toBe('')
    }
  })

  it('says the extension was started without a bridge rather than calling nowhere', async () => {
    const { host } = hostWith({ body: '{}' }, {})
    await expect(host.status()).rejects.toThrow(/without a host bridge/)
  })

  it('names a refusal as one, so an extension meets its own manifest', async () => {
    const { host } = hostWith({ status: 403, body: 'terminal.send was not declared' })
    await expect(host.send('hello')).rejects.toBeInstanceOf(PermissionDeniedError)
  })

  it('says the host answered badly rather than failing inside the parse', async () => {
    const notJson = hostWith({ body: '<html>gateway</html>' })
    await expect(notJson.host.status()).rejects.toBeInstanceOf(HostReplyError)

    for (const body of ['null', '{"error":"nope"}', '"a string"']) {
      const noResult = hostWith({ body })
      await expect(noResult.host.status()).rejects.toThrow(/carrying no result/)
    }
  })

  it('reads an empty body as nothing, which is what a write answers with', async () => {
    const { host } = hostWith({ body: '' })
    await expect(host.send('hello')).resolves.toBeUndefined()
  })
})
