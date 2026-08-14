import { describe, it, expect, vi, afterEach } from 'vitest'
import { EventEmitter } from 'node:events'
import { callStreaming, callBidiStreaming, CALL_TIMEOUT_MS } from '../src/main/device-companion'
import type { CompanionClient } from '../src/main/device-companion'

/**
 * The two streaming shapes the companion proto uses, and why telling them apart
 * matters more than it looks.
 *
 * `hid` is `stream HIDEvent → HIDResponse` and `launch` is
 * `stream LaunchRequest → stream LaunchResponse`. grpc-js gives the first a
 * callback overload and the second none at all, so sending a bidi call through
 * the callback helper produces a stream nobody reads and a promise that never
 * settles. That is what device_launch did: no error, no log line, just an agent
 * waiting out its turn — the one failure mode with nothing to report.
 */

/** A client-streaming call: collects writes, answers through the callback. */
class WritableCall extends EventEmitter {
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
  }
}

/** A bidi call: collects writes, answers by emitting data then end. */
class DuplexCall extends WritableCall {}

afterEach(() => vi.useRealTimers())

describe('callStreaming, for stream X → Y', () => {
  it('sends every message before closing, and resolves with the one reply', async () => {
    // A tap is two events. Sending only the DOWN leaves a finger held on the
    // glass and every later gesture misbehaves for reasons nothing reports.
    const call = new WritableCall()
    const client = {
      hid: (cb: (e: null, r: unknown) => void) => {
        queueMicrotask(() => cb(null, { ok: true }))
        return call
      }
    } as unknown as CompanionClient

    await expect(callStreaming(client, 'hid', [{ down: 1 }, { up: 1 }])).resolves.toEqual({
      ok: true
    })
    expect(call.written).toEqual([{ down: 1 }, { up: 1 }])
    expect(call.ended).toBe(true)
  })

  it('survives a client that answers synchronously', async () => {
    // Real gRPC always answers on a later tick, so the deadline could be armed
    // after the call was opened and nothing would notice. A mock — or a future
    // fast path — that calls back inside the same turn would then reach the
    // timer before it existed and crash instead of resolving.
    const call = new WritableCall()
    const client = {
      hid: (cb: (e: null, r: unknown) => void) => {
        cb(null, { ok: true })
        return call
      }
    } as unknown as CompanionClient

    await expect(callStreaming(client, 'hid', [{ down: 1 }])).resolves.toEqual({ ok: true })
  })

  it('carries the status detail, which is the actionable half of a failure', async () => {
    const client = {
      hid: (cb: (e: unknown) => void) => {
        queueMicrotask(() => cb({ details: 'device is not booted', message: 'call failed' }))
        return new WritableCall()
      }
    } as unknown as CompanionClient

    await expect(callStreaming(client, 'hid', [{}])).rejects.toThrow('device is not booted')
  })
})

it('lets a caller buy more time for a hold it asked the device to make', async () => {
  // A long press is DOWN, a delay, UP — all inside one call. Charged against
  // the bare round-trip budget, a press held for as long as the deadline
  // allows loses the race with it: the press happens and the call reports
  // failure. The caller adds the hold to the budget.
  vi.useFakeTimers()
  const call = new WritableCall()
  const client = { hid: () => call } as unknown as CompanionClient

  const pending = callStreaming(client, 'hid', [{}], CALL_TIMEOUT_MS + 30_000)
  const nearlyThere = vi.advanceTimersByTimeAsync(CALL_TIMEOUT_MS + 29_000)
  let settled = false
  void pending.catch(() => {
    settled = true
  })
  await nearlyThere
  expect(settled).toBe(false)

  const rest = vi.advanceTimersByTimeAsync(2_000)
  await expect(pending).rejects.toThrow(/within 60s/)
  await rest
})

describe('callBidiStreaming, for stream X → stream Y', () => {
  it('resolves from the stream rather than a callback that never comes', async () => {
    // The regression: grpc-js generates no callback overload for a bidi method,
    // so the reply only ever arrives as stream events.
    const call = new DuplexCall()
    const client = {
      launch: () => {
        queueMicrotask(() => {
          call.emit('data', { running: true })
          call.emit('end')
        })
        return call
      }
    } as unknown as CompanionClient

    await expect(
      callBidiStreaming(client, 'launch', [{ start: { bundle_id: 'com.apple.Preferences' } }])
    ).resolves.toEqual({ running: true })
    expect(call.written).toEqual([{ start: { bundle_id: 'com.apple.Preferences' } }])
    expect(call.ended).toBe(true)
  })

  it('survives a client that answers synchronously', async () => {
    // Both helpers arm the deadline before opening the call, so neither can
    // reach the timer before it exists. Unreachable as the handlers are
    // registered today — pinned so a reordering cannot make it reachable.
    const call = new DuplexCall()
    const client = {
      launch: () => {
        call.emit('data', { running: true })
        queueMicrotask(() => call.emit('end'))
        return call
      }
    } as unknown as CompanionClient

    await expect(callBidiStreaming(client, 'launch', [{}])).resolves.toBeDefined()
  })

  it('takes the last message when the server sends several', async () => {
    const call = new DuplexCall()
    const client = {
      launch: () => {
        queueMicrotask(() => {
          call.emit('data', { stage: 'installing' })
          call.emit('data', { stage: 'running' })
          call.emit('end')
        })
        return call
      }
    } as unknown as CompanionClient

    await expect(callBidiStreaming(client, 'launch', [{}])).resolves.toEqual({ stage: 'running' })
  })

  it('settles even when the server ends without saying anything', async () => {
    const call = new DuplexCall()
    const client = {
      launch: () => {
        queueMicrotask(() => call.emit('end'))
        return call
      }
    } as unknown as CompanionClient

    await expect(callBidiStreaming(client, 'launch', [{}])).resolves.toEqual({})
  })

  it('reports an error rather than waiting for an end that will not come', async () => {
    const call = new DuplexCall()
    const client = {
      launch: () => {
        queueMicrotask(() => call.emit('error', { details: 'not installed', message: 'failed' }))
        return call
      }
    } as unknown as CompanionClient

    await expect(callBidiStreaming(client, 'launch', [{}])).rejects.toThrow('not installed')
  })

  it('gives up on a call that never answers, instead of hanging forever', async () => {
    // A silent simulator is indistinguishable from a slow one until something
    // puts a bound on it. Without this the caller waits for the life of the
    // process and the agent burns its turn.
    vi.useFakeTimers()
    const call = new DuplexCall()
    const client = { launch: () => call } as unknown as CompanionClient

    const pending = callBidiStreaming(client, 'launch', [{}])
    const advanced = vi.advanceTimersByTimeAsync(30_000)
    await expect(pending).rejects.toThrow(/did not answer launch/)
    await advanced
    // The call is cancelled too — a timed-out request left open holds the
    // companion's attention for a result nobody is waiting for any more.
    expect(call.cancelled).toBe(true)
  })
})
