import { spawn } from 'node:child_process'
import path from 'node:path'
import type {
  AgentModelCatalog,
  AgentModelChoice,
  AgentModelRequest
} from '@vornrun/shared/agent-models'
import { CURATED_MODELS, supportsModelSelection } from '@vornrun/shared/agent-models'
import type { AiAgentType, AgentCommandConfig } from '@vornrun/shared/types'
import { DEFAULT_AGENT_COMMANDS } from '@vornrun/shared/agent-defaults'
import { configManager } from './config-manager'
import { getLaunchEnv, shellEscape } from './process-utils'
import { findOnPath } from './resolve-executable'
import { assertModelCommand } from './model-arguments'

const PROBE_TIMEOUT_MS = 15_000
const CACHE_TTL_MS = 5 * 60_000
const MAX_OUTPUT_BYTES = 2 * 1024 * 1024

type Json = Record<string, unknown>
const asObject = (value: unknown): Json =>
  typeof value === 'object' && value !== null ? (value as Json) : {}

/** The agents' own answers, normalised to one shape; hidden and disabled entries dropped. */
export function parseModelChoices(agent: AiAgentType, value: unknown): AgentModelChoice[] {
  if (agent === 'opencode') {
    if (typeof value !== 'string') throw new Error('Invalid OpenCode model catalog.')
    const ids = value
      .split(/\r?\n/)
      .map((line) => line.trim())
      .filter((line) => /^[^\s/]+\/\S+$/.test(line))
    return [...new Set(ids)].map((id) => ({ id, label: id }))
  }
  if (!Array.isArray(value)) throw new Error('Invalid model catalog.')
  const byId = new Map<string, AgentModelChoice>()
  for (const item of value) {
    const entry = asObject(item)
    if (entry.hidden === true || asObject(entry.policy).state === 'disabled') continue
    const id =
      agent === 'claude' ? entry.value : agent === 'codex' ? (entry.model ?? entry.id) : entry.id
    if (typeof id !== 'string' || !id) continue
    const label = entry.displayName ?? entry.name
    byId.set(id, {
      id,
      label: typeof label === 'string' ? label : id,
      ...(typeof entry.description === 'string' && { description: entry.description })
    })
  }
  return [...byId.values()]
}

export interface ProbeContext {
  command: string
  args: string[]
  cwd: string
  env: Record<string, string>
}

type ProbedAgent = 'claude' | 'codex' | 'opencode'

/** The one invocation of each CLI that lists models without starting a conversation. */
function probeArguments(agent: ProbedAgent, configured: string[]): string[] {
  switch (agent) {
    case 'claude':
      return [
        ...configured,
        '-p',
        '--input-format',
        'stream-json',
        '--output-format',
        'stream-json',
        '--verbose',
        '--no-session-persistence',
        '--safe-mode',
        '--strict-mcp-config',
        '--tools',
        ''
      ]
    case 'codex':
      return [...configured, 'app-server', '--stdio']
    case 'opencode':
      return ['models']
  }
}

const UNSUPPORTED_ANSWER =
  'The agent returned an unsupported model response. Update the CLI or type a model id.'

/** Ask an installed CLI for its models; the process is bounded in output, time and what it may do. */
export function probeProcess(
  context: ProbeContext,
  agent: ProbedAgent
): Promise<AgentModelChoice[]> {
  return new Promise((resolve, reject) => {
    const args = probeArguments(agent, context.args)
    const viaShell = process.platform === 'win32' && /\.(cmd|bat)$/i.test(context.command)
    const child = spawn(
      viaShell ? shellEscape(context.command, 'cmd') : context.command,
      viaShell ? args.map((arg) => shellEscape(arg, 'cmd')) : args,
      {
        cwd: context.cwd,
        env: context.env,
        stdio: ['pipe', 'pipe', 'pipe'],
        windowsHide: true,
        shell: viaShell
      }
    )

    let settled = false
    let buffered = ''
    let bytes = 0
    let nextId = 2
    const collected: AgentModelChoice[] = []
    const cursors = new Set<string>()

    const finish = (error?: Error, result?: AgentModelChoice[]): void => {
      if (settled) return
      settled = true
      clearTimeout(deadline)
      child.stdin.destroy()
      child.kill()
      setTimeout(() => {
        if (child.exitCode === null && child.signalCode === null) child.kill('SIGKILL')
      }, 1000).unref()
      if (error) reject(error)
      else resolve(result ?? [])
    }
    const deadline = setTimeout(
      () => finish(new Error('Model discovery timed out. Refresh to retry or type a model id.')),
      PROBE_TIMEOUT_MS
    )
    const send = (message: unknown): void => {
      if (!settled) child.stdin.write(JSON.stringify(message) + '\n')
    }

    child.stdin.on('error', () => finish(new Error('Agent discovery connection closed.')))
    child.on('error', () =>
      finish(new Error('Could not start the configured agent. Check its executable.'))
    )
    // Drained, never returned: stderr can carry credentials.
    child.stderr.on('data', () => {})
    child.on('close', (code) => {
      if (settled) return
      if (agent === 'opencode' && code === 0) finish(undefined, parseModelChoices(agent, buffered))
      else
        finish(
          new Error(
            'Agent model discovery failed. Check the CLI installation and sign-in, then refresh.'
          )
        )
    })

    const readClaude = (msg: Json): void => {
      if (msg.type !== 'control_response') return
      const response = asObject(msg.response)
      if (response.subtype !== 'success') throw new Error('Initialization failed')
      finish(undefined, parseModelChoices('claude', asObject(response.response).models))
    }
    const readCodex = (msg: Json): void => {
      if (msg.id === 1) {
        if (msg.error) throw new Error('Initialization failed')
        send({ method: 'initialized' })
        send({ id: nextId, method: 'model/list', params: { limit: 100, includeHidden: false } })
        return
      }
      if (msg.id !== nextId) return
      if (msg.error) throw new Error('Model listing failed')
      const result = asObject(msg.result)
      collected.push(...parseModelChoices('codex', result.data))
      const cursor = result.nextCursor
      if (typeof cursor === 'string' && cursor) {
        if (cursors.has(cursor)) throw new Error('Repeated pagination cursor')
        cursors.add(cursor)
        send({
          id: ++nextId,
          method: 'model/list',
          params: { limit: 100, includeHidden: false, cursor }
        })
        return
      }
      finish(undefined, [...new Map(collected.map((choice) => [choice.id, choice])).values()])
    }

    child.stdout.on('data', (chunk: Buffer) => {
      bytes += chunk.length
      if (bytes > MAX_OUTPUT_BYTES)
        return finish(new Error('Model catalog exceeded the output limit.'))
      buffered += chunk.toString()
      if (agent === 'opencode') return
      let end: number
      while (!settled && (end = buffered.indexOf('\n')) !== -1) {
        const line = buffered.slice(0, end)
        buffered = buffered.slice(end + 1)
        try {
          const msg = asObject(JSON.parse(line))
          if (agent === 'claude') readClaude(msg)
          else readCodex(msg)
        } catch {
          finish(new Error(UNSUPPORTED_ANSWER))
        }
      }
    })

    if (agent === 'claude') {
      send({
        type: 'control_request',
        request_id: 'vorn-models',
        request: { subtype: 'initialize' }
      })
    } else if (agent === 'codex') {
      send({
        id: 1,
        method: 'initialize',
        params: { clientInfo: { name: 'vorn_model_catalog', version: '1.0.0' } }
      })
    } else {
      child.stdin.end()
    }
  })
}

interface CacheEntry {
  choices?: AgentModelChoice[]
  fetchedAt: number
  pending?: Promise<AgentModelCatalog>
  error?: string
}

type Discover = (
  request: AgentModelRequest,
  config: AgentCommandConfig
) => Promise<AgentModelChoice[]>

const unavailable = (error: string): AgentModelCatalog => ({
  choices: [],
  status: 'unavailable',
  error
})

/** One list per agent, command configuration and project, kept for five minutes; the stale list stands in while a refresh runs. */
export function createModelCatalogService(discover: Discover, now: () => number = Date.now) {
  const cache = new Map<string, CacheEntry>()
  return async (
    request: AgentModelRequest,
    config: AgentCommandConfig
  ): Promise<AgentModelCatalog> => {
    if (!supportsModelSelection(request.agentType)) {
      return unavailable('Model selection is unavailable for this agent.')
    }
    if (request.remoteHostId) {
      return unavailable(
        'Models on a remote host cannot be listed; use the default or type a model id.'
      )
    }
    if (!request.projectPath || request.projectPath.includes('{{')) {
      return unavailable('Choose a project to list models, or type a model id.')
    }
    const curated = CURATED_MODELS[request.agentType]
    if (curated) return { choices: curated, status: 'ready', source: 'built-in' }

    const key = JSON.stringify([request.agentType, path.resolve(request.projectPath), config])
    const entry = cache.get(key) ?? { fetchedAt: 0 }
    cache.set(key, entry)
    const fresh = entry.choices && now() - entry.fetchedAt < CACHE_TTL_MS
    if (!request.refresh && fresh) {
      return {
        choices: entry.choices!,
        status: 'ready',
        source: 'agent',
        fetchedAt: entry.fetchedAt
      }
    }
    entry.pending ??= discover(request, config)
      .then((choices): AgentModelCatalog => {
        entry.choices = choices
        entry.fetchedAt = now()
        entry.error = undefined
        return { choices, status: 'ready', source: 'agent', fetchedAt: entry.fetchedAt }
      })
      .catch((error: unknown): AgentModelCatalog => {
        entry.error =
          error instanceof Error
            ? error.message
            : 'Could not list models. Check the agent installation and sign-in, then refresh or type a model id.'
        return {
          choices: entry.choices ?? [],
          status: entry.choices ? 'stale' : 'unavailable',
          source: entry.choices ? 'agent' : undefined,
          fetchedAt: entry.choices ? entry.fetchedAt : undefined,
          error: entry.error
        }
      })
      .finally(() => {
        entry.pending = undefined
      })
    if (!request.refresh && entry.choices) {
      return {
        choices: entry.choices,
        status: 'stale',
        source: 'agent',
        fetchedAt: entry.fetchedAt,
        error: entry.error
      }
    }
    return entry.pending
  }
}

/** Only the selectors that shape which models a CLI can see; a prompt or a mode would start work. */
function discoveryArguments(agent: AiAgentType, configured: string[]): string[] {
  if (agent !== 'codex') return []
  const paired = ['-c', '--config', '-p', '--profile', '--local-provider']
  const args: string[] = []
  for (let i = 0; i < configured.length; i++) {
    const arg = configured[i]!
    if (paired.includes(arg) && configured[i + 1]) args.push(arg, configured[++i]!)
    else if (paired.some((flag) => arg.startsWith(flag + '='))) args.push(arg)
    else if (arg === '--oss') args.push(arg)
  }
  return args
}

const catalog = createModelCatalogService(async (request, config) => {
  const env = getLaunchEnv()
  assertModelCommand(config.command)
  const command = path.isAbsolute(config.command)
    ? config.command
    : findOnPath(config.command, env.PATH ?? env.Path)
  if (!command) throw new Error(`${config.command} is not installed on this machine.`)
  return probeProcess(
    {
      command,
      args: discoveryArguments(request.agentType, config.args),
      cwd: request.projectPath,
      env
    },
    request.agentType as ProbedAgent
  )
})

export function listAgentModels(request: AgentModelRequest): Promise<AgentModelCatalog> {
  const config =
    configManager.loadConfig().agentCommands?.[request.agentType] ??
    DEFAULT_AGENT_COMMANDS[request.agentType]
  return catalog(request, config)
}
