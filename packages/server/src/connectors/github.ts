import { execFile, spawn } from 'node:child_process'
import { promisify } from 'node:util'
import type {
  VornConnector,
  ExternalItem,
  PollResult,
  ActionResult,
  ConnectorManifest,
  TaskStatus,
  ExternalItemPage
} from '@vornrun/shared/types'
import log from '../logger'
import { resolveGhPath, GhNotFoundError, getGhEnv } from './gh-cli'

const execFileAsync = promisify(execFile)

// JSON Schema subsets of the GitHub REST responses we pass through as
// `ActionResult.output`. Only keys that are useful to reference from
// downstream workflow steps are listed — the raw body still contains
// everything, the schema just drives the variable-autocomplete UI and
// documents the stable fields.
const GITHUB_ISSUE_SCHEMA: Record<string, unknown> = {
  type: 'object',
  properties: {
    id: { type: 'number', description: 'Numeric id of the issue' },
    number: { type: 'number', description: 'Issue number within the repo' },
    html_url: { type: 'string', description: 'Issue URL to show in UIs' },
    url: { type: 'string', description: 'GitHub REST API URL for the issue' },
    title: { type: 'string' },
    body: { type: 'string' },
    state: { type: 'string', description: 'open or closed' },
    labels: { type: 'array' },
    assignees: { type: 'array' },
    created_at: { type: 'string' },
    updated_at: { type: 'string' },
    closed_at: { type: 'string' }
  }
}

const GITHUB_COMMENT_SCHEMA: Record<string, unknown> = {
  type: 'object',
  properties: {
    id: { type: 'number' },
    html_url: { type: 'string', description: 'Link to the comment' },
    body: { type: 'string' },
    created_at: { type: 'string' },
    updated_at: { type: 'string' }
  }
}

const TRANSIENT_CODES = new Set(['ETIMEDOUT', 'ENETDOWN', 'ENETUNREACH', 'ECONNRESET'])

function isTransientErr(err: unknown): boolean {
  if (!err || typeof err !== 'object') return false
  const code = (err as { code?: string }).code
  return typeof code === 'string' && TRANSIENT_CODES.has(code)
}

/**
 * Run `gh` with a 15s timeout. One retry on transient network errors so a
 * flaky WiFi blip doesn't kill a poll. Captures stderr into the error message
 * so gh's non-zero exit reasons (e.g. rate limiting, auth) surface in logs.
 */
async function gh(args: string[], cwd?: string, input?: string): Promise<string> {
  const ghPath = resolveGhPath()
  if (!ghPath) throw new GhNotFoundError()
  const env = getGhEnv()
  const run = async () => {
    if (input !== undefined) {
      return runWithStdin(ghPath, args, input, cwd, env)
    }
    const { stdout } = await execFileAsync(ghPath, args, {
      timeout: 15_000,
      maxBuffer: 10 * 1024 * 1024,
      env,
      ...(cwd && { cwd })
    })
    return stdout
  }
  try {
    return await run()
  } catch (err: unknown) {
    if (isTransientErr(err)) {
      log.warn(`[github-connector] transient error, retrying once: ${String(err)}`)
      try {
        return await run()
      } catch (retryErr) {
        const msg = retryErr instanceof Error ? retryErr.message : String(retryErr)
        log.error(`[github-connector] gh command failed after retry: gh ${args.join(' ')} — ${msg}`)
        throw new Error(`gh command failed: ${msg}`, { cause: retryErr })
      }
    }
    const msg = err instanceof Error ? err.message : String(err)
    log.error(`[github-connector] gh command failed: gh ${args.join(' ')} — ${msg}`)
    throw new Error(`gh command failed: ${msg}`, { cause: err })
  }
}

/** Run `gh` feeding `input` on stdin. Used by `gh api --input -` paths so we
 *  never interpolate untrusted body values into shell arguments. */
function runWithStdin(
  ghPath: string,
  args: string[],
  input: string,
  cwd: string | undefined,
  env: Record<string, string>
): Promise<string> {
  return new Promise((resolve, reject) => {
    // Note: child_process.spawn doesn't honor `timeout` (unlike execFile), so
    // we enforce it with an explicit timer. Without this, a `gh` subprocess
    // blocking on e.g. a hung credential helper would hang poll/exec forever.
    const child = spawn(ghPath, args, { cwd, env })
    let stdout = ''
    let stderr = ''
    let settled = false
    const timer = setTimeout(() => {
      if (settled) return
      settled = true
      try {
        child.kill('SIGTERM')
      } catch {
        /* already exited */
      }
      reject(new Error('gh command timed out after 15s'))
    }, 15_000)
    child.stdout.on('data', (d) => {
      stdout += d.toString()
    })
    child.stderr.on('data', (d) => {
      stderr += d.toString()
    })
    child.on('error', (err) => {
      if (settled) return
      settled = true
      clearTimeout(timer)
      reject(err)
    })
    child.on('close', (code) => {
      if (settled) return
      settled = true
      clearTimeout(timer)
      if (code === 0) return resolve(stdout)
      reject(new Error(`gh exited with code ${code}: ${stderr.trim() || 'no stderr'}`))
    })
    child.stdin.write(input)
    child.stdin.end()
  })
}

/** Detect owner/repo from a git repo path using gh CLI */
export async function detectRepoSlug(
  projectPath: string
): Promise<{ owner: string; repo: string } | null> {
  try {
    const result = await gh(
      ['repo', 'view', '--json', 'nameWithOwner', '-q', '.nameWithOwner'],
      projectPath
    )
    const slug = result.trim()
    if (!slug.includes('/')) return null
    const [owner, repo] = slug.split('/')
    return { owner, repo }
  } catch {
    return null
  }
}

/**
 * Invoke the GitHub REST API via `gh api`. For non-GET requests with a body,
 * the JSON body is piped over stdin using `--input -`, which side-steps shell
 * escaping entirely — no interpolation of untrusted values into `-f` flags,
 * no injection surface even if the body contains quotes/semicolons/newlines.
 */
async function ghApi(
  endpoint: string,
  method = 'GET',
  body?: Record<string, unknown>
): Promise<unknown> {
  const args = ['api', endpoint]
  if (method !== 'GET') {
    args.push('-X', method)
  }
  let result: string
  if (body && Object.keys(body).length > 0) {
    args.push('--input', '-')
    result = await gh(args, undefined, JSON.stringify(body))
  } else {
    result = await gh(args)
  }
  return result.trim() ? JSON.parse(result) : null
}

interface GitHubIssue {
  number: number
  title: string
  body: string | null
  state: string
  html_url: string
  updated_at: string
  created_at: string
  labels: Array<{ name: string }>
  assignee: { login: string } | null
  pull_request?: unknown
  user?: { login: string }
}

interface GitHubSearchResponse {
  total_count: number
  incomplete_results: boolean
  items: GitHubIssue[]
}

interface GitHubPollCursor {
  since: string
  page: number
}

const GITHUB_POLL_PAGE_SIZE = 100
const GITHUB_SEARCH_MAX_PAGE = 10
const GITHUB_SEARCH_INDEX_OVERLAP_MS = 5 * 60_000

function parsePollCursor(cursor: string | undefined): GitHubPollCursor {
  if (cursor) {
    try {
      const parsed = JSON.parse(cursor) as Partial<GitHubPollCursor>
      if (
        typeof parsed.since === 'string' &&
        typeof parsed.page === 'number' &&
        Number.isInteger(parsed.page) &&
        parsed.page > 0
      ) {
        return { since: parsed.since, page: parsed.page }
      }
    } catch {
      // Legacy cursors were plain ISO timestamps.
    }
    return { since: cursor, page: 1 }
  }
  return { since: new Date(Date.now() - 60_000).toISOString(), page: 1 }
}

function searchCursor(cursor: GitHubPollCursor): string {
  return JSON.stringify(cursor)
}

function githubSearchTimestamp(value: string): string {
  const date = new Date(value)
  if (Number.isNaN(date.getTime()))
    throw new Error(`Invalid GitHub poll cursor timestamp: ${value}`)
  // GitHub compares at second precision with a strict lower bound. Replay the
  // previous second so items sharing the cursor's second cannot be skipped.
  return new Date(date.getTime() - 1_000).toISOString().replace(/\.\d{3}Z$/, 'Z')
}

function githubSearchEndpoint(
  owner: string,
  repo: string,
  kind: 'issue' | 'pr',
  cursor: GitHubPollCursor,
  labels: unknown
): string {
  const terms = [
    `repo:${owner}/${repo}`,
    `is:${kind}`,
    `created:>${githubSearchTimestamp(cursor.since)}`
  ]
  if (kind === 'issue' && typeof labels === 'string') {
    for (const label of labels
      .split(',')
      .map((value) => value.trim())
      .filter(Boolean)) {
      terms.push(`label:"${label.replaceAll('"', '\\"')}"`)
    }
  }
  const params = new URLSearchParams({
    q: terms.join(' '),
    sort: 'created',
    order: 'asc',
    per_page: String(GITHUB_POLL_PAGE_SIZE),
    page: String(cursor.page)
  })
  return `search/issues?${params}`
}

function nextSearchCursor(
  current: GitHubPollCursor,
  response: GitHubSearchResponse,
  pollStartedAt: string
): { cursor: string; hasMore: boolean } {
  const hasAnotherSearchPage = response.total_count > current.page * GITHUB_POLL_PAGE_SIZE
  if (!hasAnotherSearchPage) {
    return {
      cursor: new Date(
        new Date(pollStartedAt).getTime() - GITHUB_SEARCH_INDEX_OVERLAP_MS
      ).toISOString(),
      hasMore: false
    }
  }

  if (current.page < GITHUB_SEARCH_MAX_PAGE) {
    return {
      cursor: searchCursor({ since: current.since, page: current.page + 1 }),
      hasMore: true
    }
  }

  // GitHub Search caps accessible results at 1,000. Advance the time window
  // and start again instead of attempting page 11. Overlap one second so
  // equal-timestamp items are replayed into the deduplicating durable inbox
  // rather than skipped at the boundary.
  const lastTimestamp = response.items.at(-1)?.created_at
  if (!lastTimestamp) {
    throw new Error('GitHub search reported more results but returned an empty page')
  }
  const overlap = new Date(new Date(lastTimestamp).getTime() - 1_000).toISOString()
  if (new Date(overlap).getTime() <= new Date(current.since).getTime()) {
    throw new Error(
      'GitHub search returned more than 1,000 items at one timestamp; cannot advance safely'
    )
  }
  return { cursor: searchCursor({ since: overlap, page: 1 }), hasMore: true }
}

function assertCompleteSearch(response: GitHubSearchResponse): void {
  if (response.incomplete_results) {
    throw new Error('GitHub search returned incomplete results; retrying without advancing cursor')
  }
}

function issueToExternalItem(issue: GitHubIssue): ExternalItem {
  return {
    externalId: String(issue.number),
    url: issue.html_url,
    title: issue.title,
    description: issue.body || '',
    status: issue.state,
    labels: issue.labels?.map((l) => l.name) ?? [],
    assignee: issue.assignee?.login,
    updatedAt: issue.updated_at,
    metadata: { createdAt: issue.created_at }
  }
}

async function listGitHubItemsPage(
  filters: Record<string, unknown>,
  cursor?: string
): Promise<ExternalItemPage> {
  const { owner, repo, state = 'open', labels, assignee } = filters
  if (typeof owner !== 'string' || typeof repo !== 'string' || !owner || !repo) {
    throw new Error('owner and repo are required')
  }
  const parsedPage = Number(cursor ?? '1')
  const page = Number.isInteger(parsedPage) && parsedPage > 0 ? parsedPage : 1
  const params = new URLSearchParams({
    state: String(state),
    per_page: String(GITHUB_POLL_PAGE_SIZE),
    page: String(page)
  })
  if (labels) params.set('labels', String(labels))
  if (assignee) params.set('assignee', String(assignee))
  const endpoint = `repos/${encodeURIComponent(owner)}/${encodeURIComponent(repo)}/issues?${params}`
  const raw = (await ghApi(endpoint)) as GitHubIssue[]
  return {
    items: raw.filter((item) => !item.pull_request).map(issueToExternalItem),
    ...(raw.length === GITHUB_POLL_PAGE_SIZE && {
      nextCursor: String(page + 1),
      hasMore: true
    })
  }
}

export const githubConnector: VornConnector = {
  id: 'github',
  name: 'GitHub',
  icon: 'github',
  capabilities: ['tasks', 'triggers', 'actions'],

  async listItems(filters: Record<string, unknown>): Promise<ExternalItem[]> {
    return (await listGitHubItemsPage(filters)).items
  },

  listItemsPage: listGitHubItemsPage,

  async getItem(
    externalId: string,
    filters: Record<string, unknown>
  ): Promise<ExternalItem | null> {
    const { owner, repo } = filters
    if (typeof owner !== 'string' || typeof repo !== 'string' || !owner || !repo) {
      throw new Error('owner and repo are required')
    }

    try {
      const issue = (await ghApi(
        `repos/${encodeURIComponent(owner)}/${encodeURIComponent(repo)}/issues/${encodeURIComponent(externalId)}`
      )) as GitHubIssue
      return issueToExternalItem(issue)
    } catch {
      return null
    }
  },

  async poll(
    triggerType: string,
    config: Record<string, unknown>,
    cursor?: string
  ): Promise<PollResult> {
    const { owner, repo } = config
    if (typeof owner !== 'string' || typeof repo !== 'string' || !owner || !repo) {
      return { events: [] }
    }

    const parsedCursor = parsePollCursor(cursor)
    const pollStartedAt = new Date().toISOString()

    switch (triggerType) {
      case 'issueCreated': {
        const response = (await ghApi(
          githubSearchEndpoint(owner, repo, 'issue', parsedCursor, config.labels)
        )) as GitHubSearchResponse
        assertCompleteSearch(response)
        const next = nextSearchCursor(parsedCursor, response, pollStartedAt)
        return {
          events: response.items.map((i) => ({
            id: String(i.number),
            type: 'issueCreated',
            data: issueToExternalItem(i) as unknown as Record<string, unknown>,
            timestamp: i.created_at
          })),
          nextCursor: next.cursor,
          hasMore: next.hasMore
        }
      }
      case 'prOpened': {
        const response = (await ghApi(
          githubSearchEndpoint(owner, repo, 'pr', parsedCursor, undefined)
        )) as GitHubSearchResponse
        assertCompleteSearch(response)
        const next = nextSearchCursor(parsedCursor, response, pollStartedAt)
        return {
          events: response.items.map((pr) => ({
            id: String(pr.number),
            type: 'prOpened',
            data: {
              number: pr.number,
              title: pr.title,
              url: pr.html_url,
              description: pr.body || '',
              state: pr.state,
              author: pr.user?.login
            },
            timestamp: pr.created_at
          })),
          nextCursor: next.cursor,
          hasMore: next.hasMore
        }
      }
      default:
        return { events: [] }
    }
  },

  async execute(actionType: string, args: Record<string, unknown>): Promise<ActionResult> {
    const { owner, repo } = args
    if (!owner || !repo) return { success: false, error: 'owner and repo are required' }

    switch (actionType) {
      case 'createIssue': {
        const { title, body, labels: issueLabels } = args
        if (!title) return { success: false, error: 'title is required' }
        const bodyArgs: Record<string, unknown> = {
          title: String(title)
        }
        if (body) bodyArgs.body = String(body)
        if (issueLabels) {
          bodyArgs.labels = String(issueLabels)
        }
        const result = await ghApi(`repos/${owner}/${repo}/issues`, 'POST', bodyArgs)
        return { success: true, output: result as Record<string, unknown> }
      }
      case 'closeIssue': {
        const { number: issueNumber } = args
        if (!issueNumber) return { success: false, error: 'number is required' }
        const result = await ghApi(`repos/${owner}/${repo}/issues/${issueNumber}`, 'PATCH', {
          state: 'closed'
        })
        return { success: true, output: result as Record<string, unknown> }
      }
      case 'commentOnIssue': {
        const { number: num, body: comment } = args
        if (!num || !comment) return { success: false, error: 'number and body are required' }
        const result = await ghApi(`repos/${owner}/${repo}/issues/${num}/comments`, 'POST', {
          body: String(comment)
        })
        return { success: true, output: result as Record<string, unknown> }
      }
      case 'syncTasks': {
        // This is handled by the sync engine at a higher level.
        // The action node calls listItems() and does the upsert logic.
        return { success: true }
      }
      default:
        return { success: false, error: `Unknown action: ${actionType}` }
    }
  },

  describe(): ConnectorManifest {
    return {
      auth: [], // gh CLI handles auth — no fields needed
      taskFilters: [
        {
          key: 'state',
          label: 'State',
          type: 'select',
          options: [
            { value: 'open', label: 'Open' },
            { value: 'closed', label: 'Closed' },
            { value: 'all', label: 'All' }
          ]
        },
        { key: 'labels', label: 'Labels', type: 'text', placeholder: 'bug,enhancement' },
        { key: 'assignee', label: 'Assignee', type: 'text', placeholder: 'username or @me' }
      ],
      statusMapping: [
        { upstream: 'open', suggestedLocal: 'todo' as TaskStatus },
        { upstream: 'closed', suggestedLocal: 'done' as TaskStatus }
      ],
      triggers: [
        {
          type: 'issueCreated',
          label: 'Issue Created',
          description: 'Fires when a new issue is created',
          configFields: [
            { key: 'owner', label: 'Owner', type: 'text', required: true },
            { key: 'repo', label: 'Repository', type: 'text', required: true },
            { key: 'labels', label: 'Filter by labels', type: 'text' }
          ],
          defaultIntervalMs: 30_000
        },
        {
          type: 'prOpened',
          label: 'PR Opened',
          description: 'Fires when a new pull request is opened',
          configFields: [
            { key: 'owner', label: 'Owner', type: 'text', required: true },
            { key: 'repo', label: 'Repository', type: 'text', required: true }
          ],
          defaultIntervalMs: 30_000
        }
      ],
      actions: [
        // Note: owner/repo are sourced from the connection's filters and
        // merged in server-side before connector.execute() runs — they're
        // deliberately not duplicated in these configFields so the action
        // form stays focused on per-call args.
        {
          type: 'createIssue',
          label: 'Create Issue',
          description: 'Create a new GitHub issue in the connected repo',
          configFields: [
            {
              key: 'title',
              label: 'Title',
              type: 'text',
              required: true,
              supportsTemplates: true
            },
            { key: 'body', label: 'Body', type: 'textarea', supportsTemplates: true },
            { key: 'labels', label: 'Labels', type: 'text', placeholder: 'bug,enhancement' }
          ],
          outputSchema: GITHUB_ISSUE_SCHEMA
        },
        {
          type: 'closeIssue',
          label: 'Close Issue',
          description: 'Close an issue in the connected repo',
          configFields: [
            {
              key: 'number',
              label: 'Issue #',
              type: 'text',
              required: true,
              placeholder: '{{connectorItem.externalId}}',
              supportsTemplates: true
            }
          ],
          outputSchema: GITHUB_ISSUE_SCHEMA
        },
        {
          type: 'commentOnIssue',
          label: 'Comment on Issue',
          description: 'Post a comment on an issue in the connected repo',
          configFields: [
            {
              key: 'number',
              label: 'Issue #',
              type: 'text',
              required: true,
              placeholder: '{{connectorItem.externalId}}',
              supportsTemplates: true
            },
            {
              key: 'body',
              label: 'Comment',
              type: 'textarea',
              required: true,
              supportsTemplates: true
            }
          ],
          outputSchema: GITHUB_COMMENT_SCHEMA
        }
      ],
      defaultWorkflows: [
        {
          name: 'GitHub: Issue Created',
          event: 'issueCreated',
          defaultCronFromMinutes: 5,
          downstream: 'createTaskFromItem'
        },
        {
          name: 'GitHub: PR Opened',
          event: 'prOpened',
          defaultCronFromMinutes: 5,
          downstream: 'createTaskFromItem'
        }
      ]
    }
  }
}
