import path from 'node:path'
import log from 'electron-log/main'
import { inject } from 'vitest'
import { installSandbox } from '../helpers/sandbox'

const home = installSandbox({
  realHome: inject('realHome'),
  sandboxRoot: inject('sandboxRoot'),
  repoRoot: inject('repoRoot')
})
log.transports.file.resolvePathFn = () => path.join(home, 'logs', 'main.log')
