import os from 'node:os'
import path from 'node:path'
import { expect, inject, it } from 'vitest'
import { assertSandboxed, useRealHome } from './helpers/sandbox'

const realHome = useRealHome()

it('hands an opted-in file the real home and lifts the guard', () => {
  expect(realHome).toBe(inject('realHome'))
  expect(os.homedir()).toBe(realHome)
  expect(() => assertSandboxed(path.join(realHome, '.vorn', 'port'), 'read')).not.toThrow()
})
