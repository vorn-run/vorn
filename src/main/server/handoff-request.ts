import path from 'node:path'
import type { ServerIdentity } from '@vornrun/shared/protocol'

/**
 * Whether the running server should be replaced by this build. Its terminals
 * live in the session holder, which the replacement takes over.
 */
export type HandoffVerdict = { ask: true; why: string } | { ask: false; why: string }

/** Windows locks a running exe and cannot hand a terminal over, so an update there ends the sessions this app's server holds. */
export function updateEndsSessions(platform: NodeJS.Platform, ownsServer: boolean): boolean {
  return platform === 'win32' && ownsServer
}

export function decideHandoff(input: {
  platform: NodeJS.Platform
  incumbent: Pick<ServerIdentity, 'appVersion' | 'buildChannel'>
  self: { appVersion: string; buildChannel: 'dev' | 'packaged' }
  /** A dev build has one version across every edit, so a person must be able to say so. */
  forced?: boolean
}): HandoffVerdict {
  if (input.platform === 'win32') {
    // No way to inherit a console pseudoterminal, and no local endpoint either.
    return { ask: false, why: 'this platform cannot pass a terminal between processes' }
  }
  if (input.incumbent.buildChannel !== input.self.buildChannel) {
    // `judgeAdoption` refuses first; stated anyway because the failure would be baffling.
    return { ask: false, why: 'the running server is a different kind of build' }
  }
  if (input.forced) return { ask: true, why: 'asked for' }
  if (input.incumbent.appVersion === 'unknown') {
    // Started from the command line: not this app's to replace on a guess.
    return { ask: false, why: 'the running server did not say which build it is' }
  }
  if (input.incumbent.appVersion === input.self.appVersion) {
    return { ask: false, why: 'the running server is already this build' }
  }
  return {
    ask: true,
    why: `the running server is ${input.incumbent.appVersion} and this app is ${input.self.appVersion}`
  }
}

/**
 * Whether a running server this app could adopt is an older build to replace
 * with its own. Its terminals live in the session holder, which the new
 * server takes over, so stopping it ends none of them. Only between packaged
 * builds that both say which they are: a dev build has one version across
 * every edit.
 */
export function replacesIncumbent(
  incumbent: Pick<ServerIdentity, 'appVersion' | 'buildChannel'>,
  self: { appVersion: string; buildChannel: 'dev' | 'packaged' }
): boolean {
  return (
    incumbent.buildChannel === 'packaged' &&
    self.buildChannel === 'packaged' &&
    incumbent.appVersion !== 'unknown' &&
    incumbent.appVersion !== self.appVersion
  )
}

/** Where a dev checkout's server entry sits, relative to the built main process. */
export function devRepoRoot(dirname: string): string {
  return path.join(dirname, '../..')
}
