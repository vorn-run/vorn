import fs from 'node:fs'
import path from 'node:path'
import os from 'node:os'
import log from './logger'

/** One terminal's view of the hooks file every terminal shares; its script names the terminal from `VORN_SESSION_ID`. */
export interface CopilotHookInstallation {
  /** The `session_id` this terminal's hooks post, derived from its terminal id. */
  sessionId: string
  hooksJsonPath: string
}

// Copilot camelCase -> Vorn PascalCase event mapping
const EVENT_MAP: Record<string, string> = {
  sessionStart: 'SessionStart',
  sessionEnd: 'SessionEnd',
  userPromptSubmitted: 'Notification',
  preToolUse: 'PreToolUse',
  postToolUse: 'PostToolUse',
  errorOccurred: 'PostToolUseFailure'
}

const SESSION_PREFIX = 'copilot-'

/** The file this process wrote, removed at shutdown. */
let installedPath: string | null = null

/** Vorn's routing id for a Copilot terminal's hooks, as its script posts it. */
export function copilotHookSessionId(terminalId: string): string {
  return SESSION_PREFIX + terminalId
}

/** Copilot CLI loads every `*.json` in `$COPILOT_HOME/hooks` (default `~/.copilot/hooks`), never a project-root `hooks.json`. */
function hooksJsonPath(): string {
  const copilotHome = process.env.COPILOT_HOME || path.join(os.homedir(), '.copilot')
  return path.join(copilotHome, 'hooks', 'vorn.json')
}

// The node script is cross-platform -- only the shell invocation differs
function buildNodeScript(eventName: string): string {
  const portPath = path.join(os.homedir(), '.vorn', 'port').replace(/\\/g, '/')
  const tokenPath = path.join(os.homedir(), '.vorn', 'token').replace(/\\/g, '/')
  return [
    `const t=process.env.VORN_SESSION_ID||'';`,
    `if(!t){process.stdout.write('{}');process.exit(0)}`,
    `const d=JSON.parse(require('fs').readFileSync(0,'utf8'));`,
    `let port,token;`,
    `try{port=require('fs').readFileSync('${portPath}','utf8').trim();token=require('fs').readFileSync('${tokenPath}','utf8').trim()}catch(e){process.stdout.write('{}');process.exit(0)}`,
    `const body=JSON.stringify({session_id:'${SESSION_PREFIX}'+t,hook_event_name:'${eventName}',cwd:d.cwd||'',tool_name:d.toolName||'',vorn_terminal_id:t});`,
    `const r=require('http').request({hostname:'127.0.0.1',port:+port,path:'/hooks',method:'POST',headers:{'Content-Type':'application/json','Authorization':'Bearer '+token}});`,
    `r.on('error',()=>{});r.end(body);`,
    `process.stdout.write('{}')`
  ].join('')
}

// A Copilot outside Vorn skips starting node for every event, but still reads the event so its write never meets a closed pipe.
function buildBashCommand(script: string): string {
  return `if [ -z "$VORN_SESSION_ID" ]; then cat >/dev/null; else node -e "${script.replace(/"/g, '\\"')}"; fi`
}

function buildPowershellCommand(script: string): string {
  // PowerShell uses single quotes for the -e argument; escape internal single quotes
  return `if ($env:VORN_SESSION_ID) { node -e '${script.replace(/'/g, "''")}' }`
}

function buildHooksJson(): string {
  const hooks: Record<string, unknown[]> = {}

  for (const [copilotEvent, vornEvent] of Object.entries(EVENT_MAP)) {
    const script = buildNodeScript(vornEvent)
    hooks[copilotEvent] = [
      {
        type: 'command',
        bash: buildBashCommand(script),
        powershell: buildPowershellCommand(script)
      }
    ]
  }

  return JSON.stringify({ version: 1, _vorn: true, hooks }, null, 2)
}

/** The file's content, and whether Vorn wrote it; null when there is none. */
function readHooksFile(file: string): { content: string; ours: boolean } | null {
  let content: string
  try {
    content = fs.readFileSync(file, 'utf-8')
  } catch {
    return null
  }
  try {
    return { content, ours: JSON.parse(content)?._vorn === true }
  } catch {
    return { content, ours: false }
  }
}

/** Puts the shared hooks file in place if missing or stale, never over a file of the user's own. */
export function installCopilotHooks(terminalId: string): CopilotHookInstallation {
  const file = hooksJsonPath()
  const installation = { sessionId: copilotHookSessionId(terminalId), hooksJsonPath: file }
  const content = buildHooksJson()
  const existing = readHooksFile(file)

  if (existing && !existing.ours) {
    log.warn(`[copilot-hooks] ${file} is not Vorn's; leaving it alone`)
    return installation
  }
  if (existing?.content !== content) {
    fs.mkdirSync(path.dirname(file), { recursive: true })
    fs.writeFileSync(file, content, 'utf-8')
    log.info(`[copilot-hooks] installed ${file}`)
  }
  installedPath = file
  return installation
}

/** Removes the shared hooks file at shutdown if this process installed it and it is still Vorn's. */
export function uninstallAllCopilotHooks(): void {
  const file = installedPath
  installedPath = null
  if (!file || !readHooksFile(file)?.ours) return
  try {
    fs.unlinkSync(file)
    log.info(`[copilot-hooks] removed ${file}`)
  } catch {
    // Best-effort cleanup
  }
}
