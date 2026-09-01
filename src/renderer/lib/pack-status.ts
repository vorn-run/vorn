import type { ConnectorInstallProgress, InstalledConnectorPack } from '../../shared/types'
import type { StatusTone } from './status-tone'

/**
 * Where a connector stands between "in the catalog" and "running from disk".
 *
 * Kept apart from the components because the directory row and the detail
 * footer have to agree on what each state means, and because this mapping — not
 * the layout — is the part worth testing. A rejected install is deliberately a
 * state here rather than something persisted: nothing was written to disk, so
 * the message belongs to the session that tried it and goes away on reload.
 */
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

/**
 * Compare two versions the way a person reads them.
 *
 * Only enough to answer "is there something newer": numeric segments compared
 * left to right, and anything non-numeric — a prerelease suffix — compared as
 * text so `1.3.0` still beats `1.3.0-beta.1` rather than tying with it.
 */
export function isNewerVersion(candidate: string, current: string): boolean {
  const parts = (version: string): Array<number | string> =>
    version.split(/[.\-+]/).map((part) => (/^\d+$/.test(part) ? Number(part) : part))

  const left = parts(candidate)
  const right = parts(current)
  for (let index = 0; index < Math.max(left.length, right.length); index++) {
    const a = left[index]
    const b = right[index]
    if (a === b) continue
    // A missing segment is the release itself, which outranks any suffix it
    // could have carried: 1.3.0 is newer than 1.3.0-beta.1, not older.
    if (a === undefined) return typeof b === 'string'
    if (b === undefined) return typeof a !== 'string'
    if (typeof a === 'number' && typeof b === 'number') return a > b
    return String(a) > String(b)
  }
  return false
}

/**
 * Fold what is known about a connector into one state.
 *
 * A live install outranks everything, because it is the answer to what the row
 * is doing right now; a rejection outranks what is on disk, so a failed update
 * says so rather than quietly showing the version it failed to replace.
 */
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
        downloading: 'Downloading',
        verifying: 'Verifying',
        installing: 'Installing',
        installed: 'Installed',
        failed: 'Failed'
      }
      return {
        label: labels[state.phase],
        // The hairline below carries the number; saying it twice is two things
        // to keep in step for one fact.
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

/**
 * Whether there is anything to connect to yet.
 *
 * A built-in is already in the process and a pack is on disk, so both can be
 * connected to immediately. The third case is the one that keeps this honest:
 * a catalog entry published before packs existed still carries a package name
 * and still launches, and gating it behind an install it has no pack for would
 * strand every connector shipped so far.
 */
export function canAddConnection(
  state: PackState,
  route: { source: string; hasLegacyLaunch?: boolean }
): boolean {
  if (route.source === 'builtin') return true
  if (state.kind === 'installed') return true
  return route.hasLegacyLaunch === true
}
