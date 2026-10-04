import { describe, it, expect, vi, beforeEach } from 'vitest'
import { EventEmitter } from 'node:events'
import type { VideoMessage, VideoPort } from '../src/main/device-video'

/**
 * Main's half of the device video: open the companion's stream, relay its
 * payloads to the pane untouched, and tell the pane exactly once when the
 * stream is over, whichever way it ends.
 */

/** A bidi call: records writes, and is driven from the test. */
class DuplexCall extends EventEmitter {
  written: unknown[] = []
  ended = false
  cancelled = false
  write(m: unknown): void {
    this.written.push(m)
  }
  end(): void {
    this.ended = true
  }
  cancel(): void {
    this.cancelled = true
    // What grpc-js does: a cancelled call errors with CANCELLED.
    this.emit('error', Object.assign(new Error('Cancelled'), { code: 1, details: 'Cancelled' }))
  }
}

class Port extends EventEmitter implements VideoPort {
  messages: VideoMessage[] = []
  closed = false
  started = false
  postMessage(m: VideoMessage): void {
    if (this.closed) throw new Error('posted to a closed port')
    this.messages.push(m)
  }
  start(): void {
    this.started = true
  }
  close(): void {
    this.closed = true
  }
}

const calls: DuplexCall[] = []
const entry = {
  screenPoints: { width: 402, height: 874 } as { width: number; height: number } | null,
  scale: 3,
  companion: {
    client: {
      video_stream: () => {
        const c = new DuplexCall()
        calls.push(c)
        return c
      }
    }
  }
}
let deviceError: Error | null = null

vi.mock('../src/main/device-registry', () => ({
  deviceFor: () => {
    if (deviceError) throw deviceError
    return entry
  }
}))
vi.mock('../src/main/logger', () => ({ default: { warn: () => {} } }))

const { startVideo, stopVideo, isStreaming, videoScaleFactor, VIDEO_FPS } =
  await import('../src/main/device-video')

beforeEach(() => {
  calls.length = 0
  deviceError = null
  stopVideo('s1')
  calls.length = 0
})

describe('startVideo', () => {
  it('asks the companion for H.264 on the stream, scaled to the pane', () => {
    const port = new Port()
    startVideo({ sessionId: 's1', maxEdge: 1311 }, port)
    expect(port.started).toBe(true)
    expect(calls[0]!.written).toEqual([
      {
        start: {
          file_path: '',
          fps: VIDEO_FPS,
          format: 'H264',
          // 874 points × 3 = 2622 pixels on the long edge, asked for at 1311.
          scale_factor: 0.5,
          key_frame_rate: 2
        }
      }
    ])
  })

  it('relays each payload as it arrives, and ignores log output', () => {
    const port = new Port()
    startVideo({ sessionId: 's1' }, port)
    calls[0]!.emit('data', { payload: { data: Buffer.from([0, 0, 0, 1, 0x67]) } })
    calls[0]!.emit('data', { log_output: Buffer.from('starting') })
    calls[0]!.emit('data', { payload: { data: Buffer.from([0, 0, 1, 0x65]) } })
    expect(port.messages).toEqual([
      { type: 'data', bytes: new Uint8Array([0, 0, 0, 1, 0x67]) },
      { type: 'data', bytes: new Uint8Array([0, 0, 1, 0x65]) }
    ])
    // Plain bytes, not a Buffer that drags its pooled slab through the port.
    expect(port.messages[0]!.type === 'data' && Buffer.isBuffer(port.messages[0]!.bytes)).toBe(
      false
    )
  })

  it('tells the pane once, with the reason, when the companion fails the stream', () => {
    const port = new Port()
    startVideo({ sessionId: 's1' }, port)
    calls[0]!.emit('error', Object.assign(new Error('x'), { code: 13, details: 'encoder died' }))
    calls[0]!.emit('end')
    expect(port.messages).toEqual([{ type: 'end', error: 'encoder died' }])
    expect(port.closed).toBe(true)
    expect(isStreaming('s1')).toBe(false)
  })

  it('throws before opening anything when the session has no device', () => {
    deviceError = new Error('No device is claimed for this session.')
    expect(() => startVideo({ sessionId: 's1' }, new Port())).toThrow(/No device is claimed/)
    expect(calls).toHaveLength(0)
  })

  it('ends the previous stream when the same session starts another', () => {
    const first = new Port()
    const second = new Port()
    startVideo({ sessionId: 's1' }, first)
    startVideo({ sessionId: 's1' }, second)
    expect(calls[0]!.written.at(-1)).toEqual({ stop: {} })
    expect(calls[0]!.cancelled).toBe(true)
    expect(first.messages).toEqual([{ type: 'end', error: null }])
    expect(second.closed).toBe(false)
    expect(isStreaming('s1')).toBe(true)
  })
})

describe('stopVideo', () => {
  it('stops the encoder, cancels the call and ends the pane quietly', () => {
    const port = new Port()
    startVideo({ sessionId: 's1' }, port)
    stopVideo('s1')
    expect(calls[0]!.written.at(-1)).toEqual({ stop: {} })
    expect(calls[0]!.ended).toBe(true)
    expect(calls[0]!.cancelled).toBe(true)
    expect(port.messages).toEqual([{ type: 'end', error: null }])
    expect(isStreaming('s1')).toBe(false)
  })

  it('runs when the pane closes its end of the port', () => {
    const port = new Port()
    startVideo({ sessionId: 's1' }, port)
    port.emit('close')
    expect(calls[0]!.cancelled).toBe(true)
    expect(isStreaming('s1')).toBe(false)
  })
})

describe('videoScaleFactor', () => {
  it('scales down to the pane and never up', () => {
    expect(videoScaleFactor({ width: 402, height: 874 }, 3, 874)).toBeCloseTo(1 / 3)
    expect(videoScaleFactor({ width: 402, height: 874 }, 3, 5000)).toBe(1)
  })

  it('sends the full picture when it cannot tell', () => {
    expect(videoScaleFactor(null, 3, 800)).toBe(1)
    expect(videoScaleFactor({ width: 402, height: 874 }, 3, undefined)).toBe(1)
  })

  it('keeps a floor, so a tiny pane still gets a readable picture', () => {
    expect(videoScaleFactor({ width: 402, height: 874 }, 3, 10)).toBe(0.1)
  })
})
