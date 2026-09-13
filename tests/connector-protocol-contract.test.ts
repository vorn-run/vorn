import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import { describe, expect, expectTypeOf, it } from 'vitest'
import * as sdk from '../packages/connector-sdk/src/protocol'
import * as shared from '../packages/shared/src/connector-protocol'
import type { PollPage } from '../packages/connector-sdk/src/runtime'
import type { FooterItem, LinkHandled, NormalizedItem } from '../packages/connector-sdk/src/types'
import type { ConnectorManifest } from '../packages/connector-sdk/src/setup'

const MARKER = '// ---- protocol 1 ----'

function body(path: string): string {
  const text = readFileSync(join(__dirname, '..', path), 'utf8')
  const at = text.indexOf(MARKER)
  expect(at, `${path} has no protocol marker`).toBeGreaterThan(-1)
  return text.slice(at)
}

describe('the native connector protocol', () => {
  it('is written the same in the SDK and in the app', () => {
    expect(body('packages/shared/src/connector-protocol.ts')).toBe(
      body('packages/connector-sdk/src/protocol.ts')
    )
  })

  it('keeps the wire it was published with', () => {
    for (const side of [sdk, shared]) {
      expect(side.PROTOCOL_VERSION).toBe(1)
      expect(side.SUPPORTED_PROTOCOLS).toEqual([1])
      expect(side.MAX_FRAME_BYTES).toBe(16_777_216)
      expect(side.PROTOCOL_METHODS).toEqual({
        hello: 'vorn/hello',
        manifest: 'connector/manifest',
        preflight: 'connector/preflight',
        options: 'connector/options',
        poll: 'trigger/poll',
        action: 'action/run',
        footer: 'extension/footer',
        handler: 'extension/handler'
      })
      expect(side.PROTOCOL_ERROR_CODES).toEqual({
        methodNotFound: -32601,
        invalidParams: -32602,
        connectorError: -32000,
        unsupportedProtocol: -32001,
        beforeHello: -32002
      })
      expect(side.PROTOCOL_ERROR_KINDS).toEqual([
        'validation',
        'app-offline',
        'signed-out',
        'upstream',
        'internal'
      ])
    }
  })

  it('names a method for every method it types, and nothing else', () => {
    expectTypeOf<sdk.ProtocolMethod>().toEqualTypeOf<
      (typeof sdk.PROTOCOL_METHODS)[keyof typeof sdk.PROTOCOL_METHODS]
    >()
    expect(Object.values(sdk.PROTOCOL_METHODS)).toHaveLength(8)
  })

  it('carries what the SDK already returns, unwrapped', () => {
    expectTypeOf<PollPage>().toExtend<sdk.TriggerPollResult>()
    expectTypeOf<NormalizedItem>().toExtend<sdk.TriggerPollItem>()
    expectTypeOf<FooterItem[]>().toExtend<sdk.ExtensionFooterResult['items']>()
    expectTypeOf<LinkHandled>().toExtend<sdk.ExtensionHandlerResult>()
    expectTypeOf<ConnectorManifest & { protocol: 1 }>().toExtend<sdk.ConnectorManifestResult>()
    expect(sdk.PROTOCOL_METHODS.poll).toBe('trigger/poll')
  })

  it('types the app copy exactly as the SDK does', () => {
    expectTypeOf<shared.ProtocolMethods>().toEqualTypeOf<sdk.ProtocolMethods>()
    expectTypeOf<shared.ProtocolError>().toEqualTypeOf<sdk.ProtocolError>()
    expectTypeOf<shared.ProtocolErrorKind>().toEqualTypeOf<sdk.ProtocolErrorKind>()
    expectTypeOf<shared.ProtocolRequest>().toEqualTypeOf<sdk.ProtocolRequest>()
    expectTypeOf<shared.ProtocolResponse>().toEqualTypeOf<sdk.ProtocolResponse>()
    expectTypeOf<shared.JsonValue>().toEqualTypeOf<sdk.JsonValue>()
    expect(shared.PROTOCOL_ERROR_KINDS).toEqual(sdk.PROTOCOL_ERROR_KINDS)
  })
})
