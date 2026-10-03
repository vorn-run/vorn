import type { ExperimentalFlags } from '@vornrun/shared/types'

/**
 * Settings › Experimental, as this server last read it. Held here rather than
 * read from the config on every call, because a switch can sit on a hot path
 * (every git call reads `nativeGit`) and loading the config is a database read.
 */
let flags: ExperimentalFlags = {}

/** Called at start-up and on every config change. */
export function setExperimentalFlags(next: ExperimentalFlags | undefined): void {
  flags = { ...next }
}

export function experimentalFlag(name: keyof ExperimentalFlags): boolean {
  return flags[name] === true
}
