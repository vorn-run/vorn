import type { ServerRuntimeStatus } from '../../shared/types'

/**
 * What to say about the build holding the terminals.
 *
 * Pure and separate from the panel, like `update-status.ts`: this is what somebody
 * reads while working out why a fix they know shipped has not taken effect.
 */
export interface ServerRuntimeView {
  description: string
  /** Whether to offer the button, rather than only report. */
  offerMove: boolean
  /** What to show instead of the button. */
  trailing: string
}

export function describeServerRuntime(status: ServerRuntimeStatus): ServerRuntimeView {
  const last = status.lastUpgrade

  if (last?.kind === 'working') {
    return {
      description: 'Moving your terminals to this build. They keep running.',
      offerMove: false,
      trailing: 'Moving…'
    }
  }

  const matched = status.serverVersion === status.appVersion

  if (matched) {
    return {
      description: status.adopted
        ? `Vorn ${status.serverVersion}, running since before this window opened`
        : `Vorn ${status.serverVersion}, started by this window`,
      offerMove: false,
      trailing: 'Current'
    }
  }

  // Not broken: it is holding the terminals an update was not allowed to end.
  const behind =
    status.serverVersion === 'unknown'
      ? 'A server started outside Vorn is holding your terminals'
      : `Vorn ${status.serverVersion} is still holding your terminals; this app is ${status.appVersion}`

  const carried =
    status.sessions === null
      ? ''
      : status.sessions === 1
        ? ' Moving carries 1 terminal across without stopping it.'
        : ` Moving carries ${status.sessions} terminals across without stopping them.`

  if (!status.canUpgrade) {
    return {
      // Said plainly rather than left as a disabled button with no explanation.
      description: `${behind}. Moving it needs a restart of Vorn on this machine.`,
      offerMove: false,
      trailing: 'Restart to move'
    }
  }

  if (last?.kind === 'failed') {
    return {
      description: `${behind}. The last attempt did not take: ${last.why}`,
      // Still offered: nothing was lost, so trying again is free.
      offerMove: true,
      trailing: ''
    }
  }

  return { description: `${behind}.${carried}`, offerMove: true, trailing: '' }
}
