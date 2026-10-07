import { execFile } from 'node:child_process'

/** Runs a Copilot hook's `bash` command with `payload` on stdin; resolves with any stdin error, such as EPIPE, instead of leaving it unhandled. */
export function runCopilotHook(
  command: string,
  env: Record<string, string | undefined>,
  payload: string | Buffer
): Promise<NodeJS.ErrnoException | null> {
  return new Promise((resolve, reject) => {
    let stdinError: NodeJS.ErrnoException | null = null
    const child = execFile('sh', ['-c', command], { env: { ...process.env, ...env } }, (err) =>
      err ? reject(err) : resolve(stdinError)
    )
    child.stdin?.on('error', (err) => (stdinError = err))
    child.stdin?.end(payload)
  })
}
