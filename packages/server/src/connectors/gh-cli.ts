import type { SdkConnectorAuth } from '@vornrun/shared/types'
import { resolveExecutable } from '../resolve-executable'
import { borrowedEnv, installHintFor } from './auth-rung'

/**
 * How the built-in GitHub connector signs in, said the way a packaged one says
 * it. The built-in predates rungs and answers through its own client rather
 * than a manifest, so its declaration lives here — but it is the same shape
 * the probe and the borrow read for every other connector, which is what keeps
 * one implementation rather than a special case for `gh`.
 */
export const GH_AUTH: SdkConnectorAuth = {
  rung: 'cli',
  probe: { command: 'gh', args: ['auth', 'status'] },
  borrow: { env: ['GH_TOKEN', 'GITHUB_TOKEN'], tokenArgs: ['auth', 'token'] }
}

export function resolveGhPath(): string | null {
  return resolveExecutable('gh')
}

/**
 * Env for invoking `gh`. Starts from `getSafeEnv()` for the login-shell PATH
 * but re-adds `GH_TOKEN` / `GITHUB_TOKEN` from the raw process env — `gh`
 * supports non-interactive auth via those, and `getSafeEnv()` strips them by
 * default as a general precaution.
 */
export function getGhEnv(): Record<string, string> {
  return borrowedEnv(GH_AUTH)
}

export function ghInstallHint(): string {
  return installHintFor('gh')
}

export class GhNotFoundError extends Error {
  readonly code = 'GH_NOT_FOUND'
  constructor() {
    super(`GitHub CLI (gh) not found on PATH. ${ghInstallHint()}`)
    this.name = 'GhNotFoundError'
  }
}
