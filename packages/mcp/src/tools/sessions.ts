import { z } from 'zod'
import type { McpServer } from '@modelcontextprotocol/sdk/server/mcp.js'
import type {
  AgentType,
  CreateTerminalPayload,
  TerminalSession,
  HeadlessSession,
  RecentSession,
  SessionEvent
} from '@vornrun/shared/types'
import { V } from '../validation'
import { rpcCall, rpcNotify } from '../ws-client'

const AGENT_TYPES: [AgentType, ...AgentType[]] = [
  'claude',
  'copilot',
  'codex',
  'opencode',
  'gemini'
]

export function registerSessionTools(server: McpServer): void {
  server.tool(
    'list_sessions',
    'List terminal sessions. Filter by status: "active" (running terminals) or "recent" (past sessions).',
    {
      filter: z.enum(['active', 'recent']).optional().describe('Session filter (default: active)'),
      project_name: V.name.optional().describe('Filter by project name'),
      project_path: V.absolutePath
        .optional()
        .describe('Filter by project path (for recent sessions)')
    },
    async (args) => {
      const filter = args.filter ?? 'active'
      try {
        if (filter === 'active') {
          let sessions = await rpcCall<TerminalSession[]>('terminal:listActive')
          if (args.project_name) {
            sessions = sessions.filter((s) => s.projectName === args.project_name)
          }
          const summary = sessions.map((s) => ({
            id: s.id,
            agentType: s.agentType,
            projectName: s.projectName,
            status: s.status,
            displayName: s.displayName,
            branch: s.branch,
            pid: s.pid
          }))
          return { content: [{ type: 'text', text: JSON.stringify(summary, null, 2) }] }
        } else {
          const sessions = await rpcCall<RecentSession[]>('sessions:getRecent', args.project_path)
          return { content: [{ type: 'text', text: JSON.stringify(sessions, null, 2) }] }
        }
      } catch (err) {
        return {
          content: [{ type: 'text', text: `Error: ${err instanceof Error ? err.message : err}` }],
          isError: true
        }
      }
    }
  )

  server.tool(
    'launch_session',
    'Launch an AI agent session (interactive terminal or headless). Requires the Vorn app to be running.',
    {
      agent_type: z.enum(AGENT_TYPES).describe('Agent type to launch'),
      project_name: V.name.describe('Project name'),
      project_path: V.absolutePath.describe('Absolute path to project directory'),
      prompt: V.prompt.optional().describe('Initial prompt to send to the agent'),
      branch: V.shortText.optional().describe('Git branch to checkout'),
      use_worktree: z.boolean().optional().describe('Create a git worktree'),
      display_name: V.shortText.optional().describe('Display name for the session'),
      headless: z.boolean().optional().describe('Launch as headless (no UI) session')
    },
    async (args) => {
      const payload: CreateTerminalPayload = {
        agentType: args.agent_type as AgentType,
        projectName: args.project_name,
        projectPath: args.project_path,
        ...(args.prompt && { initialPrompt: args.prompt }),
        ...(args.branch && { branch: args.branch }),
        ...(args.use_worktree && { useWorktree: args.use_worktree }),
        ...(args.display_name && { displayName: args.display_name })
      }

      const rpcMethod = args.headless ? 'headless:create' : 'terminal:create'
      const label = args.headless ? 'headless' : 'terminal'

      try {
        const session = await rpcCall<TerminalSession | HeadlessSession>(rpcMethod, payload)
        return {
          content: [
            {
              type: 'text',
              text: JSON.stringify(
                {
                  id: session.id,
                  agentType: session.agentType,
                  projectName: session.projectName,
                  pid: session.pid,
                  status: session.status
                },
                null,
                2
              )
            }
          ]
        }
      } catch (err) {
        return {
          content: [
            {
              type: 'text',
              text: `Error launching ${label} agent: ${err instanceof Error ? err.message : err}`
            }
          ],
          isError: true
        }
      }
    }
  )

  server.tool(
    'kill_session',
    'Kill a terminal or headless session. Requires the Vorn app to be running.',
    {
      id: V.id.describe('Session ID to kill'),
      headless: z.boolean().optional().describe('Kill a headless session instead of a terminal')
    },
    async (args) => {
      const rpcMethod = args.headless ? 'headless:kill' : 'terminal:kill'
      const label = args.headless ? 'headless session' : 'session'
      try {
        await rpcCall(rpcMethod, args.id)
        return { content: [{ type: 'text', text: `Killed ${label}: ${args.id}` }] }
      } catch (err) {
        return {
          content: [
            {
              type: 'text',
              text: `Error killing ${label}: ${err instanceof Error ? err.message : err}`
            }
          ],
          isError: true
        }
      }
    }
  )

  server.tool(
    'rename_session',
    'Rename a terminal session. Changes the display name shown in the UI.',
    {
      id: V.id.describe('Session ID'),
      display_name: V.shortText.describe('New display name')
    },
    async (args) => {
      try {
        await rpcCall('terminal:rename', { id: args.id, displayName: args.display_name })
        return {
          content: [{ type: 'text', text: `Renamed session ${args.id} to "${args.display_name}"` }]
        }
      } catch (err) {
        return {
          content: [
            {
              type: 'text',
              text: `Error renaming session: ${err instanceof Error ? err.message : err}`
            }
          ],
          isError: true
        }
      }
    }
  )

  server.tool(
    'reorder_sessions',
    'Reorder terminal sessions in the grid. Provide session IDs in the desired display order.',
    {
      session_ids: z
        .array(V.id)
        .min(1, 'At least one session ID is required')
        .describe('Session IDs in desired order')
    },
    async (args) => {
      try {
        await rpcCall('terminal:reorder', args.session_ids)
        return {
          content: [
            {
              type: 'text',
              text: `Reordered ${args.session_ids.length} sessions`
            }
          ]
        }
      } catch (err) {
        return {
          content: [
            {
              type: 'text',
              text: `Error reordering sessions: ${err instanceof Error ? err.message : err}`
            }
          ],
          isError: true
        }
      }
    }
  )

  server.tool(
    'read_session_output',
    'Read terminal output from a running session. Output is stored in a rolling 1000-line buffer with ANSI codes stripped.',
    {
      id: V.id.describe('Session ID'),
      lines: z
        .number()
        .int()
        .min(1)
        .max(1000)
        .optional()
        .describe('Number of lines to read from the end (default: all)')
    },
    async (args) => {
      try {
        const output = await rpcCall<string[]>('terminal:readOutput', {
          id: args.id,
          lines: args.lines
        })
        return {
          content: [{ type: 'text', text: output.join('\n') }]
        }
      } catch (err) {
        return {
          content: [
            {
              type: 'text',
              text: `Error reading session output: ${err instanceof Error ? err.message : err}`
            }
          ],
          isError: true
        }
      }
    }
  )

  server.tool(
    'write_to_terminal',
    'Send input to a running terminal session. Requires the Vorn app to be running.',
    {
      id: V.id.describe('Session ID'),
      data: z
        .string()
        .max(50000, 'Data must be 50000 characters or less')
        .describe('Data to write (text input to send to the agent)'),
      raw: z
        .boolean()
        .optional()
        .describe('Send data as-is without appending carriage return (for raw terminal control)')
    },
    async (args) => {
      try {
        // In raw mode, send exactly what was given. Otherwise append \r to submit.
        const data = args.raw ? args.data : args.data.replace(/[\r\n]+$/, '') + '\r'
        await rpcNotify('terminal:write', { id: args.id, data })
        return { content: [{ type: 'text', text: `Wrote to session: ${args.id}` }] }
      } catch (err) {
        return {
          content: [
            {
              type: 'text',
              text: `Error writing to terminal: ${err instanceof Error ? err.message : err}`
            }
          ],
          isError: true
        }
      }
    }
  )

  // Named-key → ANSI escape sequence map
  const KEY_MAP: Record<string, string> = {
    enter: '\r',
    escape: '\x1b',
    esc: '\x1b',
    tab: '\x09',
    'shift+tab': '\x1b[Z',
    up: '\x1b[A',
    down: '\x1b[B',
    left: '\x1b[D',
    right: '\x1b[C',
    backspace: '\x7f',
    delete: '\x1b[3~',
    home: '\x1b[H',
    end: '\x1b[F',
    'ctrl+c': '\x03',
    'ctrl+d': '\x04',
    'ctrl+x': '\x18',
    'ctrl+z': '\x1a'
  }

  server.tool(
    'send_key',
    'Send a single keystroke or key combo to a terminal session without appending Enter. Use for TUI interactions like selecting menu options (1, 2, y, n), pressing Escape, Ctrl+C, arrow keys, etc.',
    {
      id: V.id.describe('Session ID'),
      key: z
        .string()
        .min(1)
        .max(20)
        .describe(
          'Key to send: single char (1, y, n), named key (enter, escape, tab, up, down, left, right, backspace, delete, home, end), or combo (ctrl+c, ctrl+d, ctrl+x, ctrl+z, shift+tab)'
        )
    },
    async (args) => {
      const key = args.key.toLowerCase().trim()

      let data = KEY_MAP[key]

      if (!data) {
        // Handle ctrl+<letter> dynamically
        const ctrlMatch = key.match(/^ctrl\+([a-z])$/)
        if (ctrlMatch) {
          data = String.fromCharCode(ctrlMatch[1].toUpperCase().charCodeAt(0) - 64)
        } else if (args.key.length === 1) {
          // Single printable character — send as-is
          data = args.key
        } else {
          return {
            content: [
              {
                type: 'text',
                text: `Unknown key: "${args.key}". Supported: single chars (1, y, n), named keys (${Object.keys(KEY_MAP).join(', ')}), or ctrl+<letter>.`
              }
            ],
            isError: true
          }
        }
      }

      try {
        await rpcNotify('terminal:write', { id: args.id, data })
        return {
          content: [{ type: 'text', text: `Sent key "${args.key}" to session: ${args.id}` }]
        }
      } catch (err) {
        return {
          content: [
            {
              type: 'text',
              text: `Error sending key to terminal: ${err instanceof Error ? err.message : err}`
            }
          ],
          isError: true
        }
      }
    }
  )

  server.tool(
    'list_session_events',
    'List session lifecycle events (created, exited, renamed). Use for post-mortem analysis and multi-agent coordination.',
    {
      session_id: V.id.optional().describe('Filter by session ID'),
      event_type: z
        .enum(['created', 'exited', 'renamed'])
        .optional()
        .describe('Filter by event type'),
      limit: z
        .number()
        .int()
        .min(1)
        .max(200)
        .optional()
        .describe('Max events to return (default: 50)')
    },
    async (args) => {
      try {
        let events: SessionEvent[]
        if (args.session_id) {
          events = await rpcCall<SessionEvent[]>('sessionEvent:listBySession', {
            sessionId: args.session_id,
            limit: args.limit ?? 50
          })
        } else {
          events = await rpcCall<SessionEvent[]>('sessionEvent:list', {
            eventType: args.event_type,
            limit: args.limit ?? 50
          })
        }
        return { content: [{ type: 'text', text: JSON.stringify(events, null, 2) }] }
      } catch (err) {
        return {
          content: [
            {
              type: 'text',
              text: `Error listing session events: ${err instanceof Error ? err.message : err}`
            }
          ],
          isError: true
        }
      }
    }
  )
}
