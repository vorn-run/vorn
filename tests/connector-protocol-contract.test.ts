import { describe, expect, expectTypeOf, it } from 'vitest'
import * as sdk from '../packages/connector-sdk/src/protocol'
import type { PollPage } from '../packages/connector-sdk/src/runtime'
import type { FooterItem, LinkHandled, NormalizedItem } from '../packages/connector-sdk/src/types'
import type { ConnectorManifest } from '../packages/connector-sdk/src/setup'

describe('the native connector protocol', () => {
  it('keeps the wire it was published with', () => {
    expect(sdk.PROTOCOL_VERSION).toBe(1)
    expect(sdk.SUPPORTED_PROTOCOLS).toEqual([1])
    expect(sdk.MAX_FRAME_BYTES).toBe(16_777_216)
    expect(sdk.PROTOCOL_METHODS).toEqual({
      hello: 'vorn/hello',
      manifest: 'connector/manifest',
      preflight: 'connector/preflight',
      options: 'connector/options',
      poll: 'trigger/poll',
      action: 'action/run',
      footer: 'extension/footer',
      handler: 'extension/handler'
    })
    expect(sdk.PROTOCOL_ERROR_CODES).toEqual({
      methodNotFound: -32601,
      invalidParams: -32602,
      connectorError: -32000,
      unsupportedProtocol: -32001,
      beforeHello: -32002
    })
    expect(sdk.PROTOCOL_ERROR_KINDS).toEqual([
      'validation',
      'app-offline',
      'signed-out',
      'upstream',
      'internal'
    ])
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

})
