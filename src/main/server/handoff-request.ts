import path from 'node:path'
import {
  HANDOFF_PROTOCOL_VERSION,
  type HandoffRequest,
  type ServerIdentity
} from '@vornrun/shared/protocol'

/**
 * Whether the running server should be replaced without stopping it.
 *
 * An update leaves the old server holding every terminal, which is what keeps
 * agents alive and also what would strand them on last month's build. Killing it
 * is never the answer; handing the PTYs over is.
 */
export type HandoffVerdict = { ask: true; why: string } | { ask: false; why: string }

export function decideHandoff(input: {
  /** The server refuses this over TCP; asked here so the app declines quietly. */
  target: string
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
  if (!input.target.startsWith('ws+unix://')) {
    return { ask: false, why: 'the running server was reached by port rather than by name' }
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

/** The incumbent cannot derive this: its bundle was replaced. See `serverProcessSpec`. */
export function buildHandoffRequest(spec: {
  exec: string
  args: string[]
  env: Record<string, string>
  cwd: string
  appVersion: string
}): HandoffRequest {
  return {
    handoffVersion: HANDOFF_PROTOCOL_VERSION,
    exec: spec.exec,
    args: spec.args,
    env: spec.env,
    cwd: spec.cwd,
    appVersion: spec.appVersion
  }
}

/** Where a dev checkout's server entry sits, relative to the built main process. */
export function devRepoRoot(dirname: string): string {
  return path.join(dirname, '../..')
}
