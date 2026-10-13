import { inject } from 'vitest'
import { installSandbox } from '../helpers/sandbox'

installSandbox({
  realHome: inject('realHome'),
  sandboxRoot: inject('sandboxRoot'),
  repoRoot: inject('repoRoot')
})
