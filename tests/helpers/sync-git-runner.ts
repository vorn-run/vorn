import { execFileSync } from 'node:child_process'
import { gitBin, type GitRunner } from '../../packages/server/src/git-runner'
import { getSafeEnv, sshExec } from '../../packages/server/src/process-utils'

/**
 * A git runner over `execFileSync`, for the tests that answer git by mocking
 * `node:child_process`: install it with `resetGitRunner(syncGitRunner)`, and
 * every git command reaches the mock with the arguments it always had.
 */
export const syncGitRunner: GitRunner = {
  mode: 'process',
  local(args, cwd, opts) {
    try {
      return Promise.resolve(
        execFileSync(gitBin(), args, {
          cwd,
          encoding: 'utf-8',
          stdio: ['pipe', 'pipe', 'pipe'],
          env: getSafeEnv(),
          timeout: opts.timeout,
          maxBuffer: opts.maxBuffer
        })
      )
    } catch (err) {
      return Promise.reject(err)
    }
  },
  remote: (host, command, opts) => sshExec(host, command, opts)
}
