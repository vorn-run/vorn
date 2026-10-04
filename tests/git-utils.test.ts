import { describe, it, expect, vi, beforeEach } from 'vitest'

vi.mock('node:child_process', () => ({
  execFileSync: vi.fn()
}))

vi.mock('node:fs', () => ({
  default: {
    mkdirSync: vi.fn(),
    existsSync: vi.fn(() => true)
  }
}))

vi.mock('node:crypto', () => ({
  default: {
    randomUUID: vi.fn(() => 'aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee')
  }
}))

import { execFileSync } from 'node:child_process'
import {
  isGitRepo,
  getGitBranch,
  listBranches,
  getGitDiffStat,
  gitCommit,
  listWorktrees,
  renameWorktreeBranch,
  renameWorktree,
  createWorktree
} from '../packages/server/src/git-utils'
import fs from 'node:fs'

const mockFs = vi.mocked(fs)

const mockExecFileSync = vi.mocked(execFileSync)

beforeEach(() => {
  vi.clearAllMocks()
})

describe('isGitRepo', () => {
  it('returns true for a git repo', async () => {
    mockExecFileSync.mockReturnValue('true\n')
    expect(await isGitRepo('/project')).toBe(true)
  })

  it('returns false for non-git directory', async () => {
    mockExecFileSync.mockImplementation(() => {
      throw new Error('not a git repo')
    })
    expect(await isGitRepo('/not-a-repo')).toBe(false)
  })

  it('returns false for unexpected output', async () => {
    mockExecFileSync.mockReturnValue('false\n')
    expect(await isGitRepo('/project')).toBe(false)
  })
})

describe('getGitBranch', () => {
  it('returns branch name', async () => {
    mockExecFileSync.mockReturnValue('main\n')
    expect(await getGitBranch('/project')).toBe('main')
  })

  it('returns null for HEAD (detached)', async () => {
    mockExecFileSync.mockReturnValue('HEAD\n')
    expect(await getGitBranch('/project')).toBeNull()
  })

  it('returns null on error', async () => {
    mockExecFileSync.mockImplementation(() => {
      throw new Error('not a git repo')
    })
    expect(await getGitBranch('/project')).toBeNull()
  })

  it('returns null for empty output', async () => {
    mockExecFileSync.mockReturnValue('')
    expect(await getGitBranch('/project')).toBeNull()
  })
})

describe('listBranches', () => {
  it('parses multi-line output', async () => {
    mockExecFileSync.mockReturnValue('main\nfeature/foo\ndev\n')
    expect(await listBranches('/project')).toEqual(['main', 'feature/foo', 'dev'])
  })

  it('returns empty array on error', async () => {
    mockExecFileSync.mockImplementation(() => {
      throw new Error()
    })
    expect(await listBranches('/project')).toEqual([])
  })

  it('returns empty for empty output', async () => {
    mockExecFileSync.mockReturnValue('')
    expect(await listBranches('/project')).toEqual([])
  })

  it('trims whitespace from branch names', async () => {
    mockExecFileSync.mockReturnValue('  main  \n  dev  \n')
    expect(await listBranches('/project')).toEqual(['main', 'dev'])
  })
})

describe('getGitDiffStat', () => {
  it('parses numstat output', async () => {
    mockExecFileSync.mockReturnValue('10\t5\tsrc/foo.ts\n3\t1\tsrc/bar.ts\n')
    expect(await getGitDiffStat('/project')).toEqual({
      filesChanged: 2,
      insertions: 13,
      deletions: 6
    })
  })

  it('handles binary files', async () => {
    mockExecFileSync.mockReturnValue('-\t-\timage.png\n5\t2\tsrc/foo.ts\n')
    expect(await getGitDiffStat('/project')).toEqual({
      filesChanged: 2,
      insertions: 5,
      deletions: 2
    })
  })

  it('returns zeros for empty diff', async () => {
    mockExecFileSync.mockReturnValue('')
    expect(await getGitDiffStat('/project')).toEqual({
      filesChanged: 0,
      insertions: 0,
      deletions: 0
    })
  })

  it('returns null on error', async () => {
    mockExecFileSync.mockImplementation(() => {
      throw new Error()
    })
    expect(await getGitDiffStat('/project')).toBeNull()
  })
})

describe('gitCommit', () => {
  it('calls git add -A when includeUnstaged is true', async () => {
    mockExecFileSync.mockReturnValue('')
    await gitCommit('/project', 'test commit', true)
    expect(mockExecFileSync).toHaveBeenCalledWith(
      'git',
      ['add', '-A'],
      expect.objectContaining({ cwd: '/project' })
    )
  })

  it('does not call git add -A when includeUnstaged is false', async () => {
    mockExecFileSync.mockReturnValue('')
    await gitCommit('/project', 'test commit', false)
    const calls = mockExecFileSync.mock.calls
    const hasAddCall = calls.some(
      (c) => c[0] === 'git' && Array.isArray(c[1]) && (c[1] as string[]).includes('-A')
    )
    expect(hasAddCall).toBe(false)
  })

  it('passes message as argument (not interpolated into command string)', async () => {
    mockExecFileSync.mockReturnValue('')
    await gitCommit('/project', 'fix: "quotes" and stuff', false)
    expect(mockExecFileSync).toHaveBeenCalledWith(
      'git',
      ['commit', '-m', 'fix: "quotes" and stuff'],
      expect.any(Object)
    )
  })

  it('returns success on successful commit', async () => {
    mockExecFileSync.mockReturnValue('')
    expect(await gitCommit('/project', 'msg', false)).toEqual({ success: true })
  })

  it('returns error on failure', async () => {
    mockExecFileSync.mockImplementation(() => {
      throw new Error('nothing to commit')
    })
    const result = await gitCommit('/project', 'msg', false)
    expect(result.success).toBe(false)
    expect(result.error).toContain('nothing to commit')
  })
})

describe('listWorktrees', () => {
  it('parses porcelain output', async () => {
    mockExecFileSync.mockReturnValue(
      'worktree /path/to/project\nbranch refs/heads/main\n\n' +
        'worktree /path/to/worktree\nbranch refs/heads/feature\n'
    )
    const result = await listWorktrees('/path/to/project')
    expect(result).toEqual([
      { path: '/path/to/project', branch: 'main', isMain: true, name: 'project' },
      { path: '/path/to/worktree', branch: 'feature', isMain: false, name: 'worktree' }
    ])
  })

  it('handles detached HEAD', async () => {
    mockExecFileSync.mockReturnValue(
      'worktree /path/to/project\nbranch refs/heads/main\n\n' +
        'worktree /path/to/worktree\ndetached\n'
    )
    const result = await listWorktrees('/path/to/project')
    expect(result[1].branch).toBe('detached')
  })

  it('returns empty on error', async () => {
    mockExecFileSync.mockImplementation(() => {
      throw new Error()
    })
    expect(await listWorktrees('/project')).toEqual([])
  })

  it('returns empty for empty output', async () => {
    mockExecFileSync.mockReturnValue('')
    expect(await listWorktrees('/project')).toEqual([])
  })
})

describe('renameWorktreeBranch', () => {
  it('renames branch when on an attached HEAD', async () => {
    // First call: getGitBranch returns current branch
    mockExecFileSync.mockReturnValueOnce('old-branch\n')
    // Second call: git branch -m succeeds
    mockExecFileSync.mockReturnValueOnce('')
    expect(await renameWorktreeBranch('/worktree', 'new-branch')).toBe(true)
    expect(mockExecFileSync).toHaveBeenCalledWith(
      'git',
      ['branch', '-m', 'new-branch'],
      expect.objectContaining({ cwd: '/worktree' })
    )
  })

  it('creates branch when on detached HEAD', async () => {
    // First call: getGitBranch throws (detached HEAD)
    mockExecFileSync.mockImplementationOnce(() => {
      throw new Error('not on a branch')
    })
    // Second call: git switch -c succeeds
    mockExecFileSync.mockReturnValueOnce('')
    expect(await renameWorktreeBranch('/worktree', 'new-branch')).toBe(true)
    expect(mockExecFileSync).toHaveBeenCalledWith(
      'git',
      ['switch', '-c', 'new-branch'],
      expect.objectContaining({ cwd: '/worktree' })
    )
  })

  it('rejects empty branch name', async () => {
    expect(await renameWorktreeBranch('/worktree', '')).toBe(false)
    expect(await renameWorktreeBranch('/worktree', '  ')).toBe(false)
    expect(mockExecFileSync).not.toHaveBeenCalled()
  })

  it('rejects option-like branch name', async () => {
    expect(await renameWorktreeBranch('/worktree', '-dangerous')).toBe(false)
    expect(mockExecFileSync).not.toHaveBeenCalled()
  })

  it('returns false on git error', async () => {
    mockExecFileSync.mockReturnValueOnce('main\n')
    mockExecFileSync.mockImplementationOnce(() => {
      throw new Error('branch already exists')
    })
    expect(await renameWorktreeBranch('/worktree', 'existing')).toBe(false)
  })
})

describe('renameWorktree', () => {
  it('renames worktree directory via git worktree move', async () => {
    mockFs.existsSync.mockReturnValue(false)
    mockExecFileSync.mockReturnValue('')
    const result = await renameWorktree('/base/old-name-abcd1234', 'new-name')
    expect(result).toEqual({ newPath: '/base/new-name-abcd1234', name: 'new-name' })
    expect(mockExecFileSync).toHaveBeenCalledWith(
      'git',
      ['worktree', 'move', '/base/old-name-abcd1234', '/base/new-name-abcd1234'],
      expect.objectContaining({ cwd: '/base/old-name-abcd1234' })
    )
  })

  it('sanitizes special characters in name', async () => {
    mockFs.existsSync.mockReturnValue(false)
    mockExecFileSync.mockReturnValue('')
    const result = await renameWorktree('/base/old-name-abcd1234', 'my cool name!')
    expect(result).toEqual({ newPath: '/base/my-cool-name-abcd1234', name: 'my-cool-name' })
  })

  it('returns null for empty name', async () => {
    expect(await renameWorktree('/base/old-abcd1234', '')).toBeNull()
    expect(await renameWorktree('/base/old-abcd1234', '  ')).toBeNull()
  })

  it('returns null if path has no short-id suffix', async () => {
    expect(await renameWorktree('/base/no-suffix', 'new-name')).toBeNull()
  })

  it('returns null if target already exists', async () => {
    mockFs.existsSync.mockReturnValue(true)
    expect(await renameWorktree('/base/old-abcd1234', 'new')).toBeNull()
  })

  it('returns null if same path after rename', async () => {
    expect(await renameWorktree('/base/same-abcd1234', 'same')).toBeNull()
  })

  it('returns null on git error', async () => {
    mockFs.existsSync.mockReturnValue(false)
    mockExecFileSync.mockImplementation(() => {
      throw new Error('git worktree move failed')
    })
    expect(await renameWorktree('/base/old-abcd1234', 'new')).toBeNull()
  })
})

describe('createWorktree', () => {
  it('uses friendly name as branch when source branch is already checked out', async () => {
    // listBranches returns 'main'
    mockExecFileSync.mockReturnValueOnce('main\n')
    // git worktree add (first attempt fails — branch checked out)
    mockExecFileSync.mockImplementationOnce(() => {
      throw new Error('already checked out')
    })
    // git worktree add -b <friendlyName> (succeeds)
    mockExecFileSync.mockReturnValueOnce('')

    const result = await createWorktree('/project', 'main', 'vivid-nova')
    expect(result.branch).toBe('vivid-nova')
    expect(result.name).toBe('vivid-nova')
    expect(mockExecFileSync).toHaveBeenCalledWith(
      'git',
      ['worktree', 'add', '-b', 'vivid-nova', expect.stringContaining('vivid-nova'), 'main'],
      expect.any(Object)
    )
  })

  it('appends shortId to branch name if friendly name already exists as branch', async () => {
    // listBranches returns both 'main' and 'vivid-nova'
    mockExecFileSync.mockReturnValueOnce('main\nvivid-nova\n')
    // git worktree add (first attempt fails)
    mockExecFileSync.mockImplementationOnce(() => {
      throw new Error('already checked out')
    })
    // git worktree add -b (succeeds)
    mockExecFileSync.mockReturnValueOnce('')

    const result = await createWorktree('/project', 'main', 'vivid-nova')
    // Should fall back to name-shortId since 'vivid-nova' branch exists
    expect(result.branch).toBe('vivid-nova-aaaaaaaa')
    expect(result.name).toBe('vivid-nova')
  })

  it('creates new branch from HEAD when branch does not exist locally', async () => {
    mockExecFileSync.mockReturnValueOnce('main\n') // listBranches
    mockExecFileSync.mockReturnValueOnce('') // git worktree add -b

    const result = await createWorktree('/project', 'feature/new', 'cosmic-flare')
    expect(result.branch).toBe('feature/new')
    expect(result.name).toBe('cosmic-flare')
  })
})
