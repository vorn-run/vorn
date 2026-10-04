import { describe, it, expect, vi } from 'vitest'
import {
  splitNals,
  accessUnits,
  codecString,
  annexB,
  nalType,
  NAL_IDR,
  NAL_SLICE,
  NAL_SPS
} from '../src/renderer/lib/h264-annexb'
import {
  DeviceVideoDecoder,
  type DecoderDeps,
  type DecoderLike
} from '../src/renderer/lib/device-video-decoder'

/**
 * What the pane does to the companion's H.264 before WebCodecs sees it: find
 * the NAL units, group them into pictures, mark key frames and read the codec
 * string. Nothing here is decoding, and every part of it is easy to get subtly
 * wrong in a way that only shows as a frozen or garbled picture.
 */

const SPS = [0x67, 0x64, 0x00, 0x1f, 0xac]
const PPS = [0x68, 0xee, 0x3c]
// The top bit of the first byte after the header is first_mb_in_slice = 0.
const IDR = [0x65, 0x88, 0x84]
const P_FIRST = [0x41, 0x9a, 0x02]
const P_SECOND = [0x41, 0x1a, 0x02]

const sc4 = [0, 0, 0, 1]
const sc3 = [0, 0, 1]
const bytes = (...parts: number[][]): Uint8Array => new Uint8Array(parts.flat())

describe('splitNals', () => {
  it('splits on three- and four-byte start codes', () => {
    const nals = splitNals(bytes(sc4, SPS, sc3, PPS, sc4, IDR))!
    expect(nals.map((n) => [...n])).toEqual([SPS, PPS, IDR])
  })

  it('keeps zero bytes that are not a start code', () => {
    const slice = [0x41, 0x00, 0x00, 0x03, 0x00, 0x01]
    expect(splitNals(bytes(sc4, slice))!.map((n) => [...n])).toEqual([slice])
  })

  it('refuses a buffer that starts in the middle of a unit', () => {
    expect(splitNals(bytes(IDR))).toBeNull()
  })
})

describe('accessUnits', () => {
  it('puts parameter sets with the key frame they precede', () => {
    const { units, rest } = accessUnits(splitNals(bytes(sc4, SPS, sc4, PPS, sc4, IDR))!)
    expect(units).toHaveLength(1)
    expect(units[0]!.key).toBe(true)
    expect(units[0]!.nals.map(nalType)).toEqual([NAL_SPS, 8, NAL_IDR])
    expect(rest).toEqual([])
  })

  it('starts a new picture at a slice whose first macroblock is 0', () => {
    const { units } = accessUnits(splitNals(bytes(sc4, P_FIRST, sc4, P_SECOND, sc4, P_FIRST))!)
    // Two slices of one picture, then the next picture.
    expect(units.map((u) => u.nals.length)).toEqual([2, 1])
    expect(units.every((u) => !u.key)).toBe(true)
    expect(nalType(units[1]!.nals[0]!)).toBe(NAL_SLICE)
  })

  it('carries parameter sets with no picture yet to the next payload', () => {
    const { units, rest } = accessUnits(splitNals(bytes(sc4, SPS, sc4, PPS))!)
    expect(units).toEqual([])
    expect(rest.map(nalType)).toEqual([NAL_SPS, 8])
  })
})

describe('codecString', () => {
  it('reads profile, constraints and level from the SPS', () => {
    expect(codecString(new Uint8Array(SPS))).toBe('avc1.64001f')
  })

  it('is null for anything that is not an SPS', () => {
    expect(codecString(new Uint8Array(PPS))).toBeNull()
  })
})

it('annexB puts each unit back behind a four-byte start code', () => {
  expect([...annexB([new Uint8Array(PPS), new Uint8Array(IDR)])]).toEqual([
    ...sc4,
    ...PPS,
    ...sc4,
    ...IDR
  ])
})

/** A decoder that records what it is fed and hands back a fake frame per chunk. */
function fakeCodecs(supported = true) {
  const chunks: Array<{ type: string; timestamp: number; data: number[] }> = []
  const configs: VideoDecoderConfig[] = []
  let output: ((f: VideoFrame) => void) | null = null
  let error: ((e: DOMException) => void) | null = null
  const decoder: DecoderLike & {
    state: 'unconfigured' | 'configured' | 'closed'
    decodeQueueSize: number
  } = {
    state: 'unconfigured',
    decodeQueueSize: 0,
    configure: (c) => {
      configs.push(c)
      decoder.state = 'configured'
    },
    decode: (chunk) => {
      const c = chunk as unknown as { type: string; timestamp: number; data: Uint8Array }
      chunks.push({ type: c.type, timestamp: c.timestamp, data: [...c.data] })
      output?.({ close: vi.fn() } as unknown as VideoFrame)
    },
    close: () => {
      decoder.state = 'closed'
    }
  }
  const deps: DecoderDeps = {
    createDecoder: (init) => {
      output = init.output
      error = init.error
      return decoder
    },
    createChunk: (init) => init as unknown as EncodedVideoChunk,
    isSupported: async () => supported
  }
  return { deps, decoder, chunks, configs, fail: (m: string) => error?.(new DOMException(m)) }
}

const flush = (): Promise<void> => new Promise((r) => setTimeout(r, 0))
const keyPayload = bytes(sc4, SPS, sc4, PPS, sc4, IDR)
const deltaPayload = bytes(sc4, P_FIRST)

describe('DeviceVideoDecoder', () => {
  it('configures from the first SPS and decodes the key frame it came with', async () => {
    const c = fakeCodecs()
    const frames: VideoFrame[] = []
    const d = new DeviceVideoDecoder(
      c.deps,
      (f) => frames.push(f),
      () => {}
    )
    d.push(keyPayload)
    await flush()
    expect(c.configs).toEqual([{ codec: 'avc1.64001f', optimizeForLatency: true }])
    expect(c.chunks).toEqual([
      { type: 'key', timestamp: 0, data: [...sc4, ...SPS, ...sc4, ...PPS, ...sc4, ...IDR] }
    ])
    d.push(deltaPayload)
    expect(c.chunks[1]).toMatchObject({ type: 'delta', timestamp: 33_333 })
    expect(frames).toHaveLength(2)
  })

  it('drops pictures until the first key frame', async () => {
    const c = fakeCodecs()
    const d = new DeviceVideoDecoder(
      c.deps,
      () => {},
      () => {}
    )
    d.push(deltaPayload)
    d.push(keyPayload)
    await flush()
    expect(c.chunks.map((x) => x.type)).toEqual(['key'])
    expect(d.dropped).toBe(1)
  })

  it('waits for the next key frame after a payload that starts mid-unit', async () => {
    const c = fakeCodecs()
    const d = new DeviceVideoDecoder(
      c.deps,
      () => {},
      () => {}
    )
    d.push(keyPayload)
    await flush()
    d.push(bytes(P_FIRST))
    d.push(deltaPayload)
    expect(c.chunks.map((x) => x.type)).toEqual(['key'])
    d.push(keyPayload)
    expect(c.chunks.map((x) => x.type)).toEqual(['key', 'key'])
  })

  it('gives up, once, on a codec the display cannot decode', async () => {
    const c = fakeCodecs(false)
    const errors: string[] = []
    const d = new DeviceVideoDecoder(
      c.deps,
      () => {},
      (m) => errors.push(m)
    )
    d.push(keyPayload)
    await flush()
    d.push(keyPayload)
    await flush()
    expect(errors).toEqual(['This display cannot decode avc1.64001f.'])
    expect(c.chunks).toEqual([])
  })

  it('reports a decode error and ignores everything after it', async () => {
    const c = fakeCodecs()
    const errors: string[] = []
    const d = new DeviceVideoDecoder(
      c.deps,
      () => {},
      (m) => errors.push(m)
    )
    d.push(keyPayload)
    await flush()
    c.fail('bad bitstream')
    d.push(keyPayload)
    expect(errors).toEqual(['bad bitstream'])
    expect(c.chunks).toHaveLength(1)
    expect(c.decoder.state).toBe('closed')
  })

  it('skips to the next key frame when the decoder falls behind', async () => {
    const c = fakeCodecs()
    const d = new DeviceVideoDecoder(
      c.deps,
      () => {},
      () => {}
    )
    d.push(keyPayload)
    await flush()
    c.decoder.decodeQueueSize = 20
    d.push(deltaPayload)
    c.decoder.decodeQueueSize = 0
    d.push(deltaPayload)
    expect(c.chunks.map((x) => x.type)).toEqual(['key'])
    d.push(keyPayload)
    expect(c.chunks.map((x) => x.type)).toEqual(['key', 'key'])
  })
})
