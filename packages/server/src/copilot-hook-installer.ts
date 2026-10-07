import fs from 'node:fs'
import path from 'node:path'
import crypto from 'node:crypto'
import os from 'node:os'
import log from './logger'

export interface CopilotHookInstallation {
  projectPath: string
  sessionId: string
  hooksJsonPath: string
  hadExistingFile: boolean
  existingContent?: string
}

// Track all active installations for bulk cleanup
const activeInstallations = new Map<string, CopilotHookInstallation>()

// Copilot camelCase -> Vorn PascalCase event mapping
const EVENT_MAP: Record<string, string> = {
  sessionStart: 'SessionStart',
  sessionEnd: 'SessionEnd',
  userPromptSubmitted: 'Notification',
  preToolUse: 'PreToolUse',
  postToolUse: 'PostToolUse',
  errorOccurred: 'PostToolUseFailure'
}

// The node script is cross-platform -- only the shell invocation differs
function buildNodeScript(sessionId: string, eventName: string): string {
  const portPath = path.join(os.homedir(), '.vorn', 'port').replace(/\\/g, '/')
  const tokenPath = path.join(os.homedir(), '.vorn', 'token').replace(/\\/g, '/')
  return [
    `const d=JSON.parse(require('fs').readFileSync(0,'utf8'));`,
    `let port,token;`,
    `try{port=require('fs').readFileSync('${portPath}','utf8').trim();token=require('fs').readFileSync('${tokenPath}','utf8').trim()}catch(e){process.stdout.write('{}');process.exit(0)}`,
    `const body=JSON.stringify({session_id:'${sessionId}',hook_event_name:'${eventName}',cwd:d.cwd||'',tool_name:d.toolName||'',vorn_terminal_id:process.env.VORN_SESSION_ID||''});`,
    `const r=require('http').request({hostname:'127.0.0.1',port:+port,path:'/hooks',method:'POST',headers:{'Content-Type':'application/json','Authorization':'Bearer '+token}});`,
    `r.on('error',()=>{});r.end(body);`,
    `process.stdout.write('{}')`
  ].join('')
}

function buildBashCommand(script: string): string {
  return `node -e "${script.replace(/"/g, '\\"')}"`
}

function buildPowershellCommand(script: string): string {
  // PowerShell uses single quotes for the -e argument; escape internal single quotes
  return `node -e '${script.replace(/'/g, "''")}'`
}

function buildHooksJson(sessionId: string): string {
  const hooks: Record<string, unknown[]> = {}

  for (const [copilotEvent, vornEvent] of Object.entries(EVENT_MAP)) {
    const script = buildNodeScript(sessionId, vornEvent)
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

export function installCopilotHooks(projectPath: string, _port: number): CopilotHookInstallation {
  const sessionId = crypto.randomUUID()
  const hooksJsonPath = path.join(projectPath, 'hooks.json')

  let hadExistingFile = false
  let existingContent: string | undefined

  // Back up existing hooks.json if present and not Vorn-managed
  try {
    if (fs.existsSync(hooksJsonPath)) {
      const content = fs.readFileSync(hooksJsonPath, 'utf-8')
      const parsed = JSON.parse(content)
      if (parsed._vorn) {
        // Already a Vorn file -- overwrite without backup
        hadExistingFile = false
      } else {
        hadExistingFile = true
        existingContent = content
      }
    }
  } catch {
    // If we can't read/parse it, treat as no existing file
    hadExistingFile = false
  }

  // Write hooks.json
  fs.writeFileSync(hooksJsonPath, buildHooksJson(sessionId), 'utf-8')

  const installation: CopilotHookInstallation = {
    projectPath,
    sessionId,
    hooksJsonPath,
    hadExistingFile,
    existingContent
  }

  activeInstallations.set(projectPath, installation)
  log.info(`[copilot-hooks] installed hooks.json at ${hooksJsonPath} (session: ${sessionId})`)

  return installation
}

export function uninstallCopilotHooks(installation: CopilotHookInstallation): void {
  try {
    if (installation.hadExistingFile && installation.existingContent) {
      // Restore original content
      fs.writeFileSync(installation.hooksJsonPath, installation.existingContent, 'utf-8')
      log.info(`[copilot-hooks] restored original hooks.json at ${installation.hooksJsonPath}`)
    } else {
      // Only remove if it's still our file
      if (fs.existsSync(installation.hooksJsonPath)) {
        const content = fs.readFileSync(installation.hooksJsonPath, 'utf-8')
        const parsed = JSON.parse(content)
        if (parsed._vorn) {
          fs.unlinkSync(installation.hooksJsonPath)
          log.info(`[copilot-hooks] removed hooks.json at ${installation.hooksJsonPath}`)
        }
      }
    }
  } catch {
    // Best-effort cleanup
  }

  activeInstallations.delete(installation.projectPath)
}

export function uninstallAllCopilotHooks(): void {
  for (const installation of activeInstallations.values()) {
    uninstallCopilotHooks(installation)
  }
  activeInstallations.clear()
}
