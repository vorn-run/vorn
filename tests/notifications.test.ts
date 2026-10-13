import { describe, it, expect } from 'vitest'
import {
  shouldNotifyStatus,
  shouldNotifyBell
} from '../packages/web/src/renderer/lib/notifications'
import type { AppConfig } from '../packages/web/src/shared/types'

function makeConfig(overrides: Partial<AppConfig['defaults']['notifications']> = {}): AppConfig {
  return {
    version: 1,
    defaults: {
      shell: '/bin/zsh',
      fontSize: 13,
      theme: 'dark',
      notifications: {
        enabled: true,
        onWaiting: true,
        onError: true,
        onBell: true,
        ...overrides
      }
    },
    projects: []
  }
}

describe('shouldNotifyStatus', () => {
  it('returns false when config is null', () => {
    expect(shouldNotifyStatus(null, 'running', 'waiting')).toBe(false)
  })

  it('returns false when notifications disabled', () => {
    const config = makeConfig({ enabled: false })
    expect(shouldNotifyStatus(config, 'running', 'waiting')).toBe(false)
  })

  it('returns true for running → waiting transition', () => {
    const config = makeConfig()
    expect(shouldNotifyStatus(config, 'running', 'waiting')).toBe(true)
  })

  it('returns false for waiting → waiting (no change)', () => {
    const config = makeConfig()
    expect(shouldNotifyStatus(config, 'waiting', 'waiting')).toBe(false)
  })

  it('returns true for running → error transition', () => {
    const config = makeConfig()
    expect(shouldNotifyStatus(config, 'running', 'error')).toBe(true)
  })

  it('returns false for error → error (no change)', () => {
    const config = makeConfig()
    expect(shouldNotifyStatus(config, 'error', 'error')).toBe(false)
  })

  it('respects onWaiting: false', () => {
    const config = makeConfig({ onWaiting: false })
    expect(shouldNotifyStatus(config, 'running', 'waiting')).toBe(false)
  })

  it('respects onError: false', () => {
    const config = makeConfig({ onError: false })
    expect(shouldNotifyStatus(config, 'running', 'error')).toBe(false)
  })

  it('returns false for running → running', () => {
    const config = makeConfig()
    expect(shouldNotifyStatus(config, 'running', 'running')).toBe(false)
  })

  it('returns false for running → idle', () => {
    const config = makeConfig()
    expect(shouldNotifyStatus(config, 'running', 'idle')).toBe(false)
  })
})

describe('shouldNotifyBell', () => {
  it('returns false when config is null', () => {
    expect(shouldNotifyBell(null)).toBe(false)
  })

  it('returns true when enabled and onBell not false', () => {
    const config = makeConfig()
    expect(shouldNotifyBell(config)).toBe(true)
  })

  it('returns false when onBell is false', () => {
    const config = makeConfig({ onBell: false })
    expect(shouldNotifyBell(config)).toBe(false)
  })

  it('returns false when notifications disabled', () => {
    const config = makeConfig({ enabled: false })
    expect(shouldNotifyBell(config)).toBe(false)
  })
})
