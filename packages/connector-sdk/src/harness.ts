import { drainPoll, runAction, runPoll, type PollPage, type RunPollOptions } from './runtime'
import { connectorManifest, type ConnectorManifest } from './setup'
import type { Connector, ConnectorConfig, NormalizedItem } from './types'

export interface HarnessOptions {
  config?: ConnectorConfig
  /** Fixed clock, so `updatedAt` defaults and cursors are deterministic. */
  now?: () => string
  /** Answer the connector's calls from the test rather than the network. */
  fetchImpl?: typeof fetch
  /** Fake clock for backoff, so a retry test costs no real time. */
  sleep?: (ms: number) => Promise<void>
}

export interface ConnectorHarness {
  poll(triggerType: string, options?: RunPollOptions): Promise<PollPage>
  drain(triggerType: string, options?: RunPollOptions): Promise<NormalizedItem[]>
  execute(actionType: string, args?: Record<string, unknown>): Promise<Record<string, unknown>>
  manifest(): ConnectorManifest
  /**
   * Poll repeatedly the way Vorn does — carrying the newest `updatedAt`
   * forward as the watermark — and return only items a real installation
   * would treat as new. Catches the classic connector bug where a poll
   * ignores its lower bound and re-delivers the same backlog forever.
   */
  pollTwice(triggerType: string, options?: RunPollOptions): Promise<NormalizedItem[]>
}

/**
 * Run a connector in-process, exactly as the MCP server would, without
 * spawning anything. Authors get real assertions in a plain unit test.
 */
export function createConnectorHarness(
  connector: Connector,
  harnessOptions: HarnessOptions = {}
): ConnectorHarness {
  const defaults = (options: RunPollOptions = {}): RunPollOptions => ({
    ...(harnessOptions.config && { config: harnessOptions.config }),
    ...(harnessOptions.now && { now: harnessOptions.now }),
    ...(harnessOptions.fetchImpl && { fetchImpl: harnessOptions.fetchImpl }),
    ...(harnessOptions.sleep && { sleep: harnessOptions.sleep }),
    ...options
  })

  return {
    poll: (triggerType, options) => runPoll(connector, triggerType, defaults(options)),
    drain: (triggerType, options) => drainPoll(connector, triggerType, defaults(options)),
    execute: (actionType, args = {}) =>
      runAction(connector, actionType, args, {
        ...(harnessOptions.config && { config: harnessOptions.config }),
        ...(harnessOptions.now && { now: harnessOptions.now }),
        ...(harnessOptions.fetchImpl && { fetchImpl: harnessOptions.fetchImpl }),
        ...(harnessOptions.sleep && { sleep: harnessOptions.sleep })
      }),
    manifest: () => connectorManifest(connector),
    async pollTwice(triggerType, options) {
      const first = await drainPoll(connector, triggerType, defaults(options))
      const watermark = first.reduce<string | undefined>(
        (newest, item) =>
          newest === undefined || item.updatedAt > newest ? item.updatedAt : newest,
        options?.since
      )
      const second = await drainPoll(
        connector,
        triggerType,
        defaults({ ...options, ...(watermark !== undefined && { since: watermark }) })
      )
      // Vorn drops anything at or before the watermark, so only strictly
      // newer items count as redelivered work.
      return watermark === undefined ? second : second.filter((item) => item.updatedAt > watermark)
    }
  }
}
