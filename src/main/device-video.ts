import type * as grpc from '@grpc/grpc-js'
import { deviceFor } from './device-registry'
import log from './logger'

/**
 * The device pane's picture as H.264 instead of polled stills.
 *
 * The companion encodes the simulator's screen itself (`video_stream`, format
 * H264), so main only relays: each payload goes to the pane as it arrives,
 * still compressed, and the renderer decodes it with WebCodecs. Nothing here
 * decodes or resizes, which is the whole point. A still costs main a full PNG
 * decode and resize; a video payload costs a copy.
 *
 * One stream per session. Starting a second one for the same session ends the
 * first, since a pane only ever draws one picture.
 */

/** What the pane hears on its port. */
export type VideoMessage =
  | { type: 'data'; bytes: Uint8Array }
  | { type: 'end'; error: string | null }

/** The port the pane listens on: Electron's `MessagePortMain`, or a test's stand-in. */
export interface VideoPort {
  postMessage(message: VideoMessage): void
  on(event: 'close', listener: () => void): unknown
  start(): void
  close(): void
}

/** Frames per second asked of the encoder. It only sends a frame when the screen changes. */
export const VIDEO_FPS = 30
/** Seconds between key frames, so a decoder that lost its place recovers quickly. */
export const KEY_FRAME_SECONDS = 2

interface Running {
  stream: grpc.ClientDuplexStream<unknown, VideoStreamResponse>
  /** Tells the pane once, however many ways the stream ends at the same time. */
  end: (error: string | null) => void
}

interface VideoStreamResponse {
  log_output?: Buffer
  payload?: { data?: Buffer }
}

const running = new Map<string, Running>()

/**
 * How much to scale the device's pixels down so the encoded picture is about
 * as large as the pane draws it. The encoder works in the screen's own pixels
 * (points × `scale`), and a 3× phone at full size is far more than any pane
 * shows.
 */
export function videoScaleFactor(
  screen: { width: number; height: number } | null,
  pixelsPerPoint: number,
  maxEdge: number | undefined
): number {
  if (!screen || !maxEdge || !(maxEdge > 0)) return 1
  const longest = Math.max(screen.width, screen.height) * (pixelsPerPoint > 0 ? pixelsPerPoint : 3)
  if (!(longest > 0)) return 1
  return Math.min(1, Math.max(0.1, maxEdge / longest))
}

/**
 * Start streaming a session's screen to `port`.
 *
 * Throws, before anything is sent, when the session has no device; after that
 * every failure arrives on the port as `end` with its message, so the pane can
 * go back to stills.
 */
export function startVideo(params: { sessionId: string; maxEdge?: number }, port: VideoPort): void {
  const entry = deviceFor(params.sessionId)
  stopVideo(params.sessionId)

  const stream = entry.companion.client.video_stream() as grpc.ClientDuplexStream<
    unknown,
    VideoStreamResponse
  >
  let ended = false
  const end = (error: string | null): void => {
    if (ended) return
    ended = true
    if (running.get(params.sessionId) === run) running.delete(params.sessionId)
    try {
      port.postMessage({ type: 'end', error })
      port.close()
    } catch {
      // The pane's end is gone already.
    }
  }
  const run: Running = { stream, end }
  running.set(params.sessionId, run)

  stream.on('data', (msg: VideoStreamResponse) => {
    const data = msg.payload?.data
    // A copy into a plain Uint8Array: the port clones what it is given, and a
    // pooled Buffer would drag its whole slab across.
    if (data && data.length > 0) port.postMessage({ type: 'data', bytes: new Uint8Array(data) })
  })
  stream.on('error', (err: grpc.ServiceError) => {
    // Cancelling is how a stream is stopped, so that one is not news.
    if (err.code === 1) end(null)
    else {
      log.warn(
        `[device] video stream for ${params.sessionId} failed: ${err.details || err.message}`
      )
      end(err.details || err.message)
    }
  })
  stream.on('end', () => end(null))
  // The pane closed its end: it unmounted, or went back to stills.
  port.on('close', () => {
    if (running.get(params.sessionId) === run) stopVideo(params.sessionId)
  })
  port.start()

  stream.write({
    start: {
      // Empty: the bytes come back on the stream rather than into a file.
      file_path: '',
      fps: VIDEO_FPS,
      format: 'H264',
      scale_factor: videoScaleFactor(entry.screenPoints, entry.scale, params.maxEdge),
      key_frame_rate: KEY_FRAME_SECONDS
    }
  })
}

/** Stop a session's stream, if it has one. */
export function stopVideo(sessionId: string): void {
  const run = running.get(sessionId)
  if (!run) return
  running.delete(sessionId)
  try {
    run.stream.write({ stop: {} })
    run.stream.end()
  } catch {
    // Already closed underneath us.
  }
  run.stream.cancel()
  run.end(null)
}

/** Whether a session is streaming, for tests. */
export function isStreaming(sessionId: string): boolean {
  return running.has(sessionId)
}
