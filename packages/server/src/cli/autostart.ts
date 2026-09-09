import { spawn } from 'node:child_process'
import fs from 'node:fs'
import path from 'node:path'
import { dataDir } from '../rpc-client'
import type { RpcTransport } from './transport'

/** The name the desktop app already writes its server output to. */
const SERVER_LOG_FILENAME = 'server.log'

const READY_TIMEOUT_MS = 20_000
const POLL_MS = 150

const sleep = (ms: number): Promise<void> => new Promise((resolve) => setTimeout(resolve, ms))

/** How this binary re-invokes itself. Dev runs from TypeScript, which needs a loader. */
function serveCommand(dataDirFlag?: string): { command: string; args: string[] } {
  const entry = process.argv[1] ?? ''
  const serve = ['server', 'serve', ...(dataDirFlag ? ['--data-dir', dataDirFlag] : [])]
  return entry.endsWith('.ts')
    ? { command: 'npx', args: ['tsx', entry, ...serve] }
    : { command: process.execPath, args: [entry, ...serve] }
}

/**
 * A server to talk to, started if there is not one already.
 *
 * Detached, so it outlives the command that needed it — the same bargain the
 * desktop app makes, and the reason `serve` refuses to idle-shutdown. A rival
 * started by a second `vorn` at the same moment stands down by itself with
 * `EXIT_ENDPOINT_TAKEN`, so no lock is held here.
 */
export async function ensureServer(
  rpc: RpcTransport,
  writeErr: (text: string) => void,
  dataDirFlag?: string
): Promise<boolean> {
  if (rpc.isRunning()) return true

  writeErr('No server running, starting one.\n')

  // The flag wins over what discovery resolved, so the log this names and the
  // directory the server is told to use cannot disagree with each other.
  const dir = dataDirFlag ?? dataDir()
  fs.mkdirSync(dir, { recursive: true })
  // A file descriptor rather than a pipe: the parent exits in a moment, and a
  // pipe dying under the server takes the server with it on its next log line.
  const logFd = fs.openSync(path.join(dir, SERVER_LOG_FILENAME), 'a')

  try {
    const { command, args } = serveCommand(dataDirFlag)
    const child = spawn(command, args, {
      stdio: ['ignore', logFd, logFd],
      detached: true,
      cwd: dir,
      env: process.env
    })
    child.unref()
  } finally {
    fs.closeSync(logFd)
  }

  const deadline = Date.now() + READY_TIMEOUT_MS
  while (Date.now() < deadline) {
    await sleep(POLL_MS)
    if (rpc.isRunning()) return true
  }

  writeErr(
    `The server did not come up within ${Math.round(READY_TIMEOUT_MS / 1000)}s. ` +
      `Its output is in ${path.join(dir, SERVER_LOG_FILENAME)}.\n`
  )
  return false
}
