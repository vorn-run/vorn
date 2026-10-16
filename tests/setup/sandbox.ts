import { afterAll, inject } from 'vitest'
import { leftOver, tiedTo } from '../helpers/daemons'
import { installSandbox } from '../helpers/sandbox'

installSandbox({
  realHome: inject('realHome'),
  sandboxRoot: inject('sandboxRoot'),
  repoRoot: inject('repoRoot')
})

// A daemon this file's worker started that is still running ends here, and fails the file that left it.
afterAll(async () => {
  if (process.platform === 'win32') return
  const left = await leftOver(() => tiedTo(process.pid), 5)
  if (left) throw new Error(left)
})
