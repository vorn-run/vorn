import { describe, it, expect, vi, beforeEach } from 'vitest'
import { EventEmitter } from 'node:events'

vi.mock('../packages/server/src/logger', () => ({
  default: { info: vi.fn(), warn: vi.fn(), error: vi.fn() }
}))

// execFile(gh, args, opts, cb) is what listItems/getItem/poll use for GETs.
// spawn(gh, args) is used only by the stdin-JSON path for POST/PATCH.
const execFileMock = vi.fn()
const spawnMock = vi.fn()

vi.mock('node:child_process', async (importOriginal) => {
  const actual = (await importOriginal()) as Record<string, unknown>
  return { ...actual, execFile: execFileMock, spawn: spawnMock }
})

// promisify(execFile) makes it (args...) => Promise<{stdout, stderr}>,
// so our mock should accept (cmd, args, opts, cb) and call cb.
function setExecFileResponse(stdout: string): void {
  execFileMock.mockImplementation(
    (
      _cmd: string,
      _args: string[],
      _opts: unknown,
      cb: (err: unknown, result: unknown) => void
    ) => {
      cb(null, { stdout, stderr: '' })
    }
  )
}

function setExecFileError(err: Error & { code?: string }): void {
  execFileMock.mockImplementation(
    (_cmd: string, _args: string[], _opts: unknown, cb: (err: unknown) => void) => {
      cb(err)
    }
  )
}

interface FakeChild extends EventEmitter {
  stdout: EventEmitter
  stderr: EventEmitter
  stdin: { write: (s: string) => void; end: () => void }
}

function makeFakeChild(stdout: string, exitCode = 0): FakeChild {
  const child = new EventEmitter() as FakeChild
  child.stdout = new EventEmitter()
  child.stderr = new EventEmitter()
  const writes: string[] = []
  child.stdin = {
    write: (s: string) => {
      writes.push(s)
    },
    end: () => {
      // Defer the data/close events so listeners are registered first.
      setImmediate(() => {
        if (stdout) child.stdout.emit('data', stdout)
        child.emit('close', exitCode)
      })
    }
  }
  ;(child as { _writes: string[] })._writes = writes
  return child
}

// Import AFTER mocks are registered.
const importGithub = async () =>
  (await import('../packages/server/src/connectors/github')).githubConnector

beforeEach(() => {
  execFileMock.mockReset()
  spawnMock.mockReset()
})

describe('github connector — describe()', () => {
  it('advertises tasks, triggers, and actions', async () => {
    const gh = await importGithub()
    expect(gh.capabilities).toEqual(['tasks', 'triggers', 'actions'])
  })

  it('declares the issueCreated and prOpened triggers', async () => {
    const gh = await importGithub()
    const manifest = gh.describe()
    const types = manifest.triggers?.map((t) => t.type)
    expect(types).toEqual(['issueCreated', 'prOpened'])
  })

  it('declares createIssue / closeIssue / commentOnIssue actions', async () => {
    const gh = await importGithub()
    const manifest = gh.describe()
    const types = manifest.actions?.map((a) => a.type)
    expect(types).toEqual(['createIssue', 'closeIssue', 'commentOnIssue'])
  })

  it('seeds both Issue Created and PR Opened workflows by default', async () => {
    const gh = await importGithub()
    const manifest = gh.describe()
    expect(manifest.defaultWorkflows?.map((e) => e.event)).toEqual(['issueCreated', 'prOpened'])
  })

  it('action configFields do not duplicate owner/repo (they come from filters)', async () => {
    const gh = await importGithub()
    const manifest = gh.describe()
    for (const action of manifest.actions ?? []) {
      const keys = action.configFields.map((f) => f.key)
      expect(keys).not.toContain('owner')
      expect(keys).not.toContain('repo')
    }
  })
})

describe('github connector — listItems()', () => {
  it('URL-encodes owner, repo, labels, assignee', async () => {
    setExecFileResponse('[]')
    const gh = await importGithub()
    await gh.listItems!({
      owner: 'owner with space',
      repo: 'repo/has/slash',
      labels: 'bug,enhancement',
      assignee: 'user+alias'
    })
    const [cmd, args] = execFileMock.mock.calls[0]
    // resolveGhPath() may return an absolute path (/usr/bin/gh, /opt/homebrew/bin/gh)
    // or the bare name when not on PATH in the test runner.
    expect(cmd).toMatch(/(?:^|[/\\])gh(?:\.(?:exe|cmd))?$/)
    const endpoint = args[1] as string
    expect(endpoint).toContain(encodeURIComponent('owner with space'))
    expect(endpoint).toContain(encodeURIComponent('repo/has/slash'))
    expect(endpoint).toContain(encodeURIComponent('bug,enhancement'))
    expect(endpoint).toContain(encodeURIComponent('user+alias'))
  })

  it('throws when owner/repo are missing or non-string', async () => {
    const gh = await importGithub()
    await expect(gh.listItems!({} as Record<string, unknown>)).rejects.toThrow(/owner and repo/)
    await expect(
      gh.listItems!({ owner: 123, repo: 'r' } as unknown as Record<string, unknown>)
    ).rejects.toThrow(/owner and repo/)
  })

  it('filters out pull requests returned by the issues endpoint', async () => {
    setExecFileResponse(
      JSON.stringify([
        {
          number: 1,
          title: 'Real issue',
          body: 'desc',
          state: 'open',
          html_url: 'https://github.com/owner/repo/issues/1',
          updated_at: '2026-01-01',
          created_at: '2026-01-01',
          labels: [],
          assignee: null
        },
        {
          number: 2,
          title: 'A PR',
          body: '',
          state: 'open',
          html_url: '',
          updated_at: '',
          created_at: '',
          labels: [],
          assignee: null,
          pull_request: {}
        }
      ])
    )
    const gh = await importGithub()
    const items = await gh.listItems!({ owner: 'owner', repo: 'repo' })
    expect(items.map((i) => i.title)).toEqual(['Real issue'])
  })

  it('exposes resumable pages for complete reconciliation', async () => {
    const raw = Array.from({ length: 100 }, (_, index) => ({
      number: index + 1,
      title: `Issue ${index + 1}`,
      body: '',
      state: 'open',
      html_url: '',
      updated_at: '2026-08-04T00:00:00Z',
      created_at: '2026-08-04T00:00:00Z',
      labels: [],
      assignee: null
    }))
    setExecFileResponse(JSON.stringify(raw))
    const gh = await importGithub()
    const page = await gh.listItemsPage!({ owner: 'o', repo: 'r' }, '2')
    expect(page.items).toHaveLength(100)
    expect(page).toMatchObject({ hasMore: true, nextCursor: '3' })
    const endpoint = execFileMock.mock.calls[0][1][1] as string
    expect(endpoint).toContain('per_page=100')
    expect(endpoint).toContain('page=2')
  })
})

describe('github connector — getItem()', () => {
  it('returns an external item on success', async () => {
    setExecFileResponse(
      JSON.stringify({
        number: 42,
        title: 'Hi',
        body: 'b',
        state: 'open',
        html_url: 'https://github.com/owner/repo/issues/42',
        updated_at: '2026-01-01',
        created_at: '2026-01-01',
        labels: [{ name: 'bug' }],
        assignee: { login: 'alice' }
      })
    )
    const gh = await importGithub()
    const item = await gh.getItem!('42', { owner: 'owner', repo: 'repo' })
    expect(item).toMatchObject({
      externalId: '42',
      title: 'Hi',
      status: 'open',
      labels: ['bug'],
      assignee: 'alice'
    })
  })

  it('returns null on any error (not found, network, etc.)', async () => {
    setExecFileError(Object.assign(new Error('404'), { code: 'HTTP_404' }))
    const gh = await importGithub()
    const item = await gh.getItem!('999', { owner: 'owner', repo: 'repo' })
    expect(item).toBeNull()
  })

  it('throws when owner/repo missing or non-string', async () => {
    const gh = await importGithub()
    await expect(gh.getItem!('1', {})).rejects.toThrow(/owner and repo/)
  })
})

describe('github connector — poll()', () => {
  it('returns empty when owner/repo are missing', async () => {
    const gh = await importGithub()
    const result = await gh.poll!('issueCreated', {})
    expect(result.events).toEqual([])
  })

  it('issueCreated: includes items closed while offline and advances when caught up', async () => {
    setExecFileResponse(
      JSON.stringify({
        total_count: 1,
        items: [
          {
            number: 1,
            title: 'New, then closed',
            body: 'b',
            state: 'closed',
            html_url: '',
            updated_at: '2026-04-24T10:06:00Z',
            created_at: '2026-04-24T10:05:00Z',
            labels: [],
            assignee: null
          }
        ]
      })
    )
    const gh = await importGithub()
    const result = await gh.poll!('issueCreated', { owner: 'o', repo: 'r' }, '2026-04-24T10:00:00Z')
    expect(result.events).toHaveLength(1)
    expect(result.events[0].id).toBe('1')
    expect(result.events[0].data.status).toBe('closed')
    expect(result.hasMore).toBe(false)
    expect(typeof result.nextCursor).toBe('string')
    expect(result.nextCursor).not.toBe('2026-04-24T10:05:00Z')
    const endpoint = execFileMock.mock.calls[0][1][1] as string
    expect(endpoint).toContain('search/issues?')
    expect(decodeURIComponent(endpoint)).toContain('is:issue')
    expect(decodeURIComponent(endpoint)).toContain('created:>2026-04-24T10:00:00Z')
  })

  it('issueCreated: returns a resumable next page instead of skipping a backlog', async () => {
    const items = Array.from({ length: 100 }, (_, i) => ({
      number: i + 1,
      title: `issue ${i + 1}`,
      body: '',
      state: 'open',
      html_url: '',
      updated_at: '',
      created_at: `2026-04-24T10:${String(Math.floor(i / 60)).padStart(2, '0')}:${String(
        i % 60
      ).padStart(2, '0')}Z`,
      labels: [],
      assignee: null
    }))
    setExecFileResponse(JSON.stringify({ total_count: 250, items }))

    const gh = await importGithub()
    const result = await gh.poll!('issueCreated', { owner: 'o', repo: 'r' }, '2026-04-24T09:59:00Z')
    expect(result.events).toHaveLength(100)
    expect(result.hasMore).toBe(true)
    expect(JSON.parse(result.nextCursor!)).toEqual({
      since: '2026-04-24T09:59:00Z',
      page: 2
    })
  })

  it('prOpened: maps PRs into trigger events', async () => {
    setExecFileResponse(
      JSON.stringify({
        total_count: 1,
        items: [
          {
            number: 7,
            title: 'Cool PR',
            body: 'description',
            state: 'merged',
            html_url: 'https://github.com/o/r/pull/7',
            updated_at: '2026-04-24T12:00:00Z',
            created_at: '2026-04-24T11:00:00Z',
            labels: [],
            assignee: null,
            user: { login: 'dev' },
            pull_request: {}
          }
        ]
      })
    )
    const gh = await importGithub()
    // Pin the `since` cursor so the test stays deterministic regardless of
    // when it runs — without a cursor the connector uses `now - 60s`, which
    // filters out the fixture PR whenever wall-clock time is past its
    // hard-coded `created_at`.
    const result = await gh.poll!('prOpened', { owner: 'o', repo: 'r' }, '2026-04-24T10:59:00Z')
    expect(result.events).toHaveLength(1)
    expect(result.events[0].data).toMatchObject({
      number: 7,
      title: 'Cool PR',
      state: 'merged',
      author: 'dev'
    })
  })

  it('resumes the exact search page encoded in the cursor', async () => {
    setExecFileResponse(JSON.stringify({ total_count: 150, items: [] }))
    const gh = await importGithub()
    await gh.poll!(
      'issueCreated',
      { owner: 'o', repo: 'r' },
      JSON.stringify({ since: '2026-04-24T10:00:00Z', page: 2 })
    )
    const endpoint = execFileMock.mock.calls[0][1][1] as string
    expect(endpoint).toContain('page=2')
  })

  it('normalizes cursor timestamps to GitHub search precision', async () => {
    setExecFileResponse(JSON.stringify({ total_count: 0, items: [] }))
    const gh = await importGithub()
    await gh.poll!('issueCreated', { owner: 'o', repo: 'r' }, '2026-04-24T10:00:00.987Z')
    const endpoint = decodeURIComponent(execFileMock.mock.calls[0][1][1] as string)
    expect(endpoint).toContain('created:>2026-04-24T10:00:00Z')
    expect(endpoint).not.toContain('.987Z')
  })

  it('unknown trigger type returns empty', async () => {
    const gh = await importGithub()
    const result = await gh.poll!('mystery', { owner: 'o', repo: 'r' })
    expect(result.events).toEqual([])
  })
})

describe('github connector — execute()', () => {
  it('returns error when owner/repo missing', async () => {
    const gh = await importGithub()
    const result = await gh.execute!('createIssue', {})
    expect(result.success).toBe(false)
    expect(result.error).toMatch(/owner and repo/)
  })

  it('createIssue requires a title', async () => {
    const gh = await importGithub()
    const result = await gh.execute!('createIssue', { owner: 'o', repo: 'r' })
    expect(result.success).toBe(false)
    expect(result.error).toMatch(/title/)
  })

  it('closeIssue requires number', async () => {
    const gh = await importGithub()
    const result = await gh.execute!('closeIssue', { owner: 'o', repo: 'r' })
    expect(result.success).toBe(false)
    expect(result.error).toMatch(/number/)
  })

  it('commentOnIssue requires number AND body', async () => {
    const gh = await importGithub()
    const result = await gh.execute!('commentOnIssue', { owner: 'o', repo: 'r', number: 1 })
    expect(result.success).toBe(false)
    expect(result.error).toMatch(/number and body/)
  })

  it('unknown action type returns error', async () => {
    const gh = await importGithub()
    const result = await gh.execute!('warpSpeed', { owner: 'o', repo: 'r' })
    expect(result.success).toBe(false)
    expect(result.error).toMatch(/Unknown action/)
  })

  it('createIssue pipes body JSON via stdin using spawn (no shell-arg injection)', async () => {
    const child = makeFakeChild('{"number":42}')
    spawnMock.mockReturnValue(child)
    const gh = await importGithub()
    const result = await gh.execute!('createIssue', {
      owner: 'o',
      repo: 'r',
      title: 'dangerous"; rm -rf /',
      body: 'with ${vars} and `backticks`'
    })
    expect(result.success).toBe(true)
    expect(spawnMock).toHaveBeenCalled()
    const args = spawnMock.mock.calls[0][1] as string[]
    expect(args).toContain('--input')
    expect(args).toContain('-')
    // Title ends up inside JSON stdin, never in argv.
    const stdinWrites = (child as unknown as { _writes: string[] })._writes
    const payload = JSON.parse(stdinWrites.join(''))
    expect(payload.title).toBe('dangerous"; rm -rf /')
  })
})
