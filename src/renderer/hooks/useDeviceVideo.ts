import { useCallback, useEffect, useRef, useState } from 'react'
import { DeviceVideoDecoder, webCodecs, type DecoderDeps } from '../lib/device-video-decoder'

/** After a failure, how long the pane stays on stills before trying video again. */
export const VIDEO_RETRY_MS = 30_000
/** Restarts allowed within `VIDEO_RETRY_MS` before a stream that keeps losing its place counts as failed. */
export const MAX_QUICK_RESTARTS = 3

export interface DeviceVideoState {
  /** A decoded picture has arrived, so the pane draws the canvas instead of stills. */
  live: boolean
  /** Hand this the canvas to draw into. It draws the latest picture as soon as it mounts. */
  canvasRef: (el: HTMLCanvasElement | null) => void
}

/**
 * The device's screen as video, decoded in the pane.
 *
 * Runs only while `enabled` (the Settings › Experimental switch), while the
 * pane can be seen, and once the pane knows the screen's size, so the stream
 * is asked for at the size the pane draws rather than the device's own. Until the first picture is decoded the pane keeps showing
 * stills, so a stream that never produces anything costs nothing but the
 * attempt. Any failure (no WebCodecs, the companion refusing the stream, a
 * picture that will not decode) puts the pane back on stills, and video is
 * tried again after `VIDEO_RETRY_MS`. A stream the decoder lost its place in
 * is restarted straight away, since a new one opens with a key frame.
 */
export function useDeviceVideo(args: {
  sessionId: string
  udid: string | null
  enabled: boolean
  onScreen: boolean
  /** The pane knows the screen's size, so `maxEdge` has an answer. */
  sized: boolean
  /** The long edge, in device pixels, the pane would draw at. */
  maxEdge: () => number | undefined
  /** For tests; the browser's WebCodecs otherwise. */
  codecs?: DecoderDeps | null
}): DeviceVideoState {
  const { sessionId, udid, enabled, onScreen, sized, maxEdge } = args
  // Made once: a new object each render would restart the stream each render.
  const [browserCodecs] = useState(() => (args.codecs === undefined ? webCodecs() : null))
  const codecs = args.codecs === undefined ? browserCodecs : args.codecs
  const [live, setLive] = useState(false)
  const [failedAt, setFailedAt] = useState<number | null>(null)
  // Bumped to open a fresh stream; the times of recent restarts bound how often.
  const [attempt, setAttempt] = useState(0)
  const restarts = useRef<number[]>([])
  const canvasEl = useRef<HTMLCanvasElement | null>(null)
  const latest = useRef<VideoFrame | null>(null)
  // Read when the stream starts, so a zoom does not restart it.
  const maxEdgeRef = useRef(maxEdge)
  useEffect(() => {
    maxEdgeRef.current = maxEdge
  })

  const draw = useCallback((frame: VideoFrame, canvas: HTMLCanvasElement) => {
    if (canvas.width !== frame.displayWidth) canvas.width = frame.displayWidth
    if (canvas.height !== frame.displayHeight) canvas.height = frame.displayHeight
    canvas.getContext('2d')?.drawImage(frame, 0, 0)
  }, [])

  const canvasRef = useCallback(
    (el: HTMLCanvasElement | null) => {
      canvasEl.current = el
      if (el && latest.current) draw(latest.current, el)
    },
    [draw]
  )

  // A different device starts over: its failures are not this one's. Reset
  // during the render that brings the new udid in, as the stills do.
  const [shown, setShown] = useState(udid)
  if (udid !== shown) {
    setShown(udid)
    setFailedAt(null)
  }

  useEffect(() => {
    if (failedAt === null) return
    const t = setTimeout(() => setFailedAt(null), VIDEO_RETRY_MS)
    return () => clearTimeout(t)
  }, [failedAt])

  const run = enabled && onScreen && sized && udid !== null && codecs !== null && failedAt === null
  const start = window.api?.deviceVideoStart

  useEffect(() => {
    if (!run || !codecs || !start) return
    let stopped = false
    let stop = (): void => {}
    const fail = (restart: boolean): void => {
      if (stopped) return
      stopped = true
      stop()
      decoder.close()
      setLive(false)
      const now = Date.now()
      restarts.current = restarts.current.filter((t) => now - t < VIDEO_RETRY_MS)
      if (restart && restarts.current.length < MAX_QUICK_RESTARTS) {
        restarts.current.push(now)
        setAttempt((n) => n + 1)
      } else {
        setFailedAt(now)
      }
    }
    const decoder = new DeviceVideoDecoder(
      codecs,
      (frame) => {
        if (stopped) {
          frame.close()
          return
        }
        latest.current?.close()
        latest.current = frame
        if (canvasEl.current) draw(frame, canvasEl.current)
        setLive(true)
      },
      (_message, how) => fail(how === 'restart')
    )
    stop = start(
      sessionId,
      maxEdgeRef.current(),
      (bytes) => decoder.push(bytes),
      // Ended without being asked to: the companion went away or refused.
      () => fail(false)
    )
    return () => {
      stopped = true
      stop()
      decoder.close()
      latest.current?.close()
      latest.current = null
      setLive(false)
    }
  }, [run, codecs, start, sessionId, udid, draw, attempt])

  return { live: run && live, canvasRef }
}
