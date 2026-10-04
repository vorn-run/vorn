import { NAL_SPS, accessUnits, annexB, codecString, nalType, splitNals } from './h264-annexb'

/** The parts of WebCodecs this uses, so tests can stand one in. */
export interface DecoderLike {
  readonly state: 'unconfigured' | 'configured' | 'closed'
  readonly decodeQueueSize: number
  configure(config: VideoDecoderConfig): void
  decode(chunk: EncodedVideoChunk): void
  close(): void
}

export interface DecoderDeps {
  createDecoder(init: VideoDecoderInit): DecoderLike
  createChunk(init: EncodedVideoChunkInit): EncodedVideoChunk
  /** Whether the browser can decode this codec string. */
  isSupported(config: VideoDecoderConfig): Promise<boolean>
}

/** A backlog this deep means the decoder is behind; drop to the next key frame. */
const MAX_QUEUE = 8

/**
 * Turns the companion's payloads into frames.
 *
 * Each payload the companion sends is one encoded picture, so a payload is
 * decoded as soon as it arrives: waiting for the next start code to prove a
 * picture complete would hold the last frame of every change until the screen
 * changed again. A payload that does not begin with a start code is the middle
 * of something, so it is dropped and decoding waits for the next key frame.
 *
 * `onFrame` owns the frame it is given and must close it. `onError` is called
 * once, when the stream cannot be decoded at all; the pane then goes back to
 * stills.
 */
export class DeviceVideoDecoder {
  private decoder: DecoderLike | null = null
  private codec: string | null = null
  private configuring: Promise<void> | null = null
  /** Configured for the newest codec string, so pictures can go straight in. */
  private ready = false
  private waitingForKey = true
  /** Parameter sets seen before the picture they belong to. */
  private carried: Uint8Array[] = []
  private timestamp = 0
  private failed = false
  /** Pictures dropped while waiting for a key frame, for diagnostics. */
  dropped = 0

  constructor(
    private readonly deps: DecoderDeps,
    private readonly onFrame: (frame: VideoFrame) => void,
    private readonly onError: (message: string) => void
  ) {}

  push(payload: Uint8Array): void {
    if (this.failed) return
    const nals = splitNals(payload)
    if (!nals) {
      this.dropped++
      this.waitingForKey = true
      this.carried = []
      return
    }
    const { units, rest } = accessUnits([...this.carried, ...nals])
    this.carried = rest
    for (const unit of units) {
      const sps = unit.nals.find((n) => nalType(n) === NAL_SPS)
      const codec = sps ? codecString(sps) : null
      if (codec && codec !== this.codec) this.configure(codec)
      if (this.waitingForKey && !unit.key) {
        this.dropped++
        continue
      }
      if (!this.ready || !this.decoder) {
        // Still configuring: keep only the newest key frame until it is.
        if (unit.key) this.queue(unit)
        else this.dropped++
        continue
      }
      if (this.decoder.decodeQueueSize > MAX_QUEUE && !unit.key) {
        this.dropped++
        this.waitingForKey = true
        continue
      }
      this.decodeUnit(unit)
    }
  }

  close(): void {
    this.failed = true
    this.ready = false
    this.pending = []
    if (this.decoder && this.decoder.state !== 'closed') this.decoder.close()
    this.decoder = null
  }

  private pending: { nals: Uint8Array[]; key: boolean }[] = []

  private queue(unit: { nals: Uint8Array[]; key: boolean }): void {
    // Only the newest key frame matters once the decoder is ready.
    this.pending = [unit]
  }

  private configure(codec: string): void {
    this.codec = codec
    this.ready = false
    const config: VideoDecoderConfig = { codec, optimizeForLatency: true }
    const run = async (): Promise<void> => {
      let supported: boolean
      try {
        supported = await this.deps.isSupported(config)
      } catch {
        supported = false
      }
      if (this.failed || this.codec !== codec) return
      if (!supported) return this.fail(`This display cannot decode ${codec}.`)
      if (!this.decoder || this.decoder.state === 'closed') {
        this.decoder = this.deps.createDecoder({
          output: (frame) => {
            if (this.failed) frame.close()
            else this.onFrame(frame)
          },
          error: (e) => this.fail(e.message)
        })
      }
      try {
        this.decoder.configure(config)
      } catch (e) {
        return this.fail(e instanceof Error ? e.message : String(e))
      }
      this.ready = true
      const pending = this.pending
      this.pending = []
      for (const unit of pending) this.decodeUnit(unit)
    }
    this.configuring = (this.configuring ?? Promise.resolve()).then(run)
  }

  private decodeUnit(unit: { nals: Uint8Array[]; key: boolean }): void {
    if (!this.decoder) return
    try {
      this.decoder.decode(
        this.deps.createChunk({
          type: unit.key ? 'key' : 'delta',
          // Microseconds, one frame apart. The decoder needs them to increase;
          // the pane draws whatever comes out, as it comes out.
          timestamp: this.timestamp,
          data: annexB(unit.nals)
        })
      )
      this.timestamp += 33_333
      if (unit.key) this.waitingForKey = false
    } catch (e) {
      this.fail(e instanceof Error ? e.message : String(e))
    }
  }

  private fail(message: string): void {
    if (this.failed) return
    this.close()
    this.onError(message)
  }
}

/** The real WebCodecs, or null where the browser has none. */
export function webCodecs(): DecoderDeps | null {
  if (typeof VideoDecoder === 'undefined' || typeof EncodedVideoChunk === 'undefined') return null
  return {
    createDecoder: (init) => new VideoDecoder(init),
    createChunk: (init) => new EncodedVideoChunk(init),
    isSupported: async (config) => (await VideoDecoder.isConfigSupported(config)).supported === true
  }
}
