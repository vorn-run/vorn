import { spawn, type ChildProcess } from 'node:child_process'
import { createInterface } from 'node:readline'

/**
 * Where a test's client connects: the server it started, or vornd in front of it.
 *
 * The RPC test files call this right after `startServer`. On an ordinary run it
 * hands back the server's own port. The conformance run (`yarn
 * test:conformance`) sets `VORN_CONFORMANCE_VORND` to a vornd binary, and then
 * every client in those files talks to the server through vornd instead, so the
 * same assertions check that vornd changes nothing a client can see.
 */
export interface FrontDoor {
  /** The port clients connect to. */
  port: number
  /** Whether that port is vornd's. */
  throughVornd: boolean
  close(): Promise<void>
}

const START_TIMEOUT_MS = 10_000

export async function frontDoor(serverPort: number): Promise<FrontDoor> {
  const binary = process.env.VORN_CONFORMANCE_VORND
  if (!binary) return { port: serverPort, throughVornd: false, close: async () => {} }

  const child = spawn(binary, ['--upstream', `127.0.0.1:${serverPort}`], {
    stdio: ['ignore', 'pipe', 'inherit'],
    env: { ...process.env, VORND_LOG: process.env.VORND_LOG ?? 'warn' }
  })
  const port = await new Promise<number>((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error('vornd did not start')), START_TIMEOUT_MS)
    const lines = createInterface({ input: child.stdout! })
    lines.once('line', (line) => {
      clearTimeout(timer)
      try {
        resolve((JSON.parse(line) as { port: number }).port)
      } catch {
        reject(new Error(`vornd printed ${line}`))
      }
    })
    child.once('exit', (code) => {
      clearTimeout(timer)
      reject(new Error(`vornd exited with ${code} before listening`))
    })
  })
  return { port, throughVornd: true, close: () => stop(child) }
}

function stop(child: ChildProcess): Promise<void> {
  if (child.exitCode !== null || child.signalCode !== null) return Promise.resolve()
  return new Promise((resolve) => {
    child.once('exit', () => resolve())
    child.kill()
  })
}
