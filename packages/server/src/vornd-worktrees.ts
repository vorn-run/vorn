/**
 * What vornd tells this server after it cleans worktrees itself.
 *
 * vornd answers the worktree manager's
 * calls and deletes what they ask. This server still measures worktrees for
 * its own answers, so it forgets the sizes of the paths vornd removed or
 * emptied, as it does after a cleanup of its own.
 */

export interface WorktreesDeps {
  channel: { on(event: 'ask', listener: (method: string, params: unknown) => void): unknown }
  forgetSize: (path: string) => void
}

export function linkWorktrees(deps: WorktreesDeps): void {
  deps.channel.on('ask', (method, params) => {
    if (method !== 'vornd:worktreesCleaned') return
    const paths = (params as { paths?: unknown } | null)?.paths
    if (!Array.isArray(paths)) return
    for (const p of paths) if (typeof p === 'string') deps.forgetSize(p)
  })
}
