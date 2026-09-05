import type { ConnectorInstallProgress, InstalledConnectorPack } from '../../shared/types'
import type { StatusTone } from './status-tone'

/** A rejection is session state, not persisted: nothing was written to disk. */
export type PackState =
  | { kind: 'absent' }
  | { kind: 'installing'; phase: ConnectorInstallProgress['phase']; percent?: number }
  | {
      kind: 'installed'
      version: string
      /** A newer published version, when the catalog names one. */
      availableVersion?: string
      /** The version a rollback would return to. */
      previousVersion?: string
    }
  | { kind: 'rejected'; error: string }

export interface PackStatusView {
  /** One line, written for a person rather than named after the phase. */
  label: string
  /** Supporting line: a version, a rejection's reason. */
  detail: string | null
  tone: StatusTone
  /** 0–100 while downloading, otherwise null. */
  percent: number | null
  /** The action worth offering, if any. */
  action: 'install' | 'update' | 'retry' | null
  /** True while the install owns the row, so its controls stay disabled. */
  busy: boolean
}

/** An installed pack launches from its own files, with nothing to resolve. */
export function packLaunch(pack: InstalledConnectorPack): { command: string; args: string[] } {
  return { command: 'node', args: [`${pack.path}/index.js`] }
}

/** Enough to answer "is there something newer", prereleases included. */
export function isNewerVersion(candidate: string, current: string): boolean {
  // Build metadata carries no precedence, so it is dropped before anything is compared.
  const split = (version: string): { release: number[]; pre: Array<number | string> } => {
    const [release, ...pre] = version.split('+')[0].split('-')
    return {
      release: release.split('.').map((part) => (/^\d+$/.test(part) ? Number(part) : 0)),
      pre: pre
        .join('-')
        .split('.')
        .filter((part) => part !== '')
        .map((part) => (/^\d+$/.test(part) ? Number(part) : part))
    }
  }

  const left = split(candidate)
  const right = split(current)
  // A missing release segment is a zero, so 1.0 and 1.0.0 are the same version.
  for (let index = 0; index < Math.max(left.release.length, right.release.length); index++) {
    const a = left.release[index] ?? 0
    const b = right.release[index] ?? 0
    if (a !== b) return a > b
  }
  // Same release: a prerelease loses to it, and to a prerelease that sorts later.
  if (left.pre.length === 0) return right.pre.length > 0
  if (right.pre.length === 0) return false
  for (let index = 0; index < Math.max(left.pre.length, right.pre.length); index++) {
    const a = left.pre[index]
    const b = right.pre[index]
    if (a === b) continue
    if (a === undefined) return false
    if (b === undefined) return true
    if (typeof a === 'number' && typeof b === 'number') return a > b
    // A numeric identifier ranks below an alphanumeric one.
    if (typeof a === 'number') return false
    if (typeof b === 'number') return true
    return a > b
  }
  return false
}

/** A live install outranks a rejection, which outranks what is on disk. */
export function packStateFor(input: {
  installed?: InstalledConnectorPack | undefined
  /** Version the catalog publishes, used only to offer an update. */
  catalogVersion?: string | undefined
  progress?: ConnectorInstallProgress | undefined
}): PackState {
  const { installed, catalogVersion, progress } = input

  if (progress && progress.phase !== 'installed' && progress.phase !== 'failed') {
    return {
      kind: 'installing',
      phase: progress.phase,
      ...(progress.percent !== undefined && { percent: progress.percent })
    }
  }
  if (progress?.phase === 'failed') {
    return { kind: 'rejected', error: progress.error ?? 'The pack could not be installed' }
  }
  if (!installed) return { kind: 'absent' }

  return {
    kind: 'installed',
    version: installed.version,
    ...(catalogVersion &&
      isNewerVersion(catalogVersion, installed.version) && { availableVersion: catalogVersion }),
    ...(installed.previousVersion !== undefined && { previousVersion: installed.previousVersion })
  }
}

export function describePackStatus(state: PackState): PackStatusView {
  switch (state.kind) {
    case 'absent':
      return {
        label: 'Install',
        detail: null,
        tone: 'idle',
        percent: null,
        action: 'install',
        busy: false
      }

    case 'installing': {
      const labels: Record<ConnectorInstallProgress['phase'], string> = {
        checking: 'Checking',
        downloading: 'Downloading',
        verifying: 'Verifying',
        installing: 'Installing',
        installed: 'Installed',
        failed: 'Failed'
      }
      return {
        label: labels[state.phase],
        // The hairline carries the number; saying it twice is two things to keep in step.
        detail: null,
        tone: 'live',
        percent: state.phase === 'downloading' ? (state.percent ?? null) : null,
        action: null,
        busy: true
      }
    }

    case 'installed':
      return state.availableVersion
        ? {
            label: 'Update',
            detail: `v${state.version} → ${state.availableVersion} available`,
            tone: 'blocked',
            percent: null,
            action: 'update',
            busy: false
          }
        : {
            label: 'Installed',
            detail: `v${state.version}`,
            tone: 'settled',
            percent: null,
            action: null,
            busy: false
          }

    case 'rejected':
      return {
        label: "Couldn't install",
        detail: state.error,
        tone: 'broken',
        percent: null,
        action: 'retry',
        busy: false
      }
  }
}

/** A catalog entry with no pack still launches by name, so it stays reachable. */
export function canAddConnection(
  state: PackState,
  route: { source: string; hasLegacyLaunch?: boolean }
): boolean {
  if (route.source === 'builtin') return true
  // An MCP server is a command, so there is nothing to install before connecting.
  if (route.source === 'mcp') return true
  if (state.kind === 'installed') return true
  return route.hasLegacyLaunch === true
}
