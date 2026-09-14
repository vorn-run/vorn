import { describe, expect, it } from 'vitest'
import { isNewerVersion } from '../src/shared/version-order'

describe('isNewerVersion', () => {
  it('compares numeric segments left to right', () => {
    expect(isNewerVersion('1.3.0', '1.2.0')).toBe(true)
    expect(isNewerVersion('1.2.0', '1.3.0')).toBe(false)
    expect(isNewerVersion('2.0.0', '1.99.99')).toBe(true)
    expect(isNewerVersion('1.10.0', '1.9.0')).toBe(true)
  })

  it('is false for the same version', () => {
    expect(isNewerVersion('1.2.0', '1.2.0')).toBe(false)
  })

  it('ranks a release above the prereleases that led to it', () => {
    expect(isNewerVersion('1.3.0', '1.3.0-beta.1')).toBe(true)
    expect(isNewerVersion('1.3.0-beta.1', '1.3.0')).toBe(false)
    expect(isNewerVersion('1.3.0-beta.2', '1.3.0-beta.1')).toBe(true)
    expect(isNewerVersion('1.3.0-rc.1', '1.3.0-beta.9')).toBe(true)
  })

  it('reads a missing segment as a zero', () => {
    expect(isNewerVersion('1.0.0', '1.0')).toBe(false)
    expect(isNewerVersion('1.0', '1.0.0')).toBe(false)
    expect(isNewerVersion('1.0.1', '1.0')).toBe(true)
  })

  it('ignores build metadata, which carries no precedence', () => {
    expect(isNewerVersion('1.0.0+build.7', '1.0.0')).toBe(false)
    expect(isNewerVersion('1.0.0', '1.0.0+build.7')).toBe(false)
    expect(isNewerVersion('1.0.1+build.7', '1.0.0')).toBe(true)
  })
})
