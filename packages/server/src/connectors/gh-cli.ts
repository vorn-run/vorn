import type { SdkConnectorAuth } from '@vornrun/shared/types'
import { resolveExecutable } from '../resolve-executable'
import { borrowedEnv, installHintFor, type BorrowSource } from './auth-rung'

// The built-in predates rungs, so its declaration lives here in the shape every packaged connector uses.
export const GH_AUTH: SdkConnectorAuth = {
  rung: 'cli',
  probe: { command: 'gh', args: ['auth', 'status'] },
  borrow: { env: ['GH_TOKEN', 'GITHUB_TOKEN'], tokenArgs: ['auth', 'token'], tokenEnv: 'GH_TOKEN' }
}

// Written by the app, so it may name a credential; what it borrows is what it declares.
export const GH_SOURCE: BorrowSource = {
  auth: GH_AUTH,
  declared: GH_AUTH.borrow?.env ?? [],
  trusted: true
}

export function resolveGhPath(): string | null {
  return resolveExecutable('gh')
}

export function getGhEnv(): Record<string, string> {
  return borrowedEnv(GH_SOURCE)
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
