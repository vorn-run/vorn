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
  borrow: { env: ['GH_TOKEN', 'GITHUB_TOKEN'], tokenArgs: ['auth', 'token'], tokenEnv: 'GH_TOKEN' }
}

/**
 * The variables this connector reads, which is what it may borrow.
 *
 * A packaged connector answers this from its own manifest. The built-in has no
 * manifest env of its own, so it says so here — the borrow is held to a
 * declaration either way rather than trusting whatever a rung asks for.
 */
export const GH_DECLARED_ENV = ['GH_TOKEN', 'GITHUB_TOKEN']

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
  return borrowedEnv(GH_AUTH, GH_DECLARED_ENV)
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
