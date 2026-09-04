import { describe, it, expect } from 'vitest'
import { loginItemFor } from '../src/main/login-item'
import type { AppConfig } from '../src/shared/types'

const withDefaults = (defaults: Partial<AppConfig['defaults']>): AppConfig =>
  ({
    version: 1,
    defaults: { shell: '/bin/zsh', fontSize: 13, theme: 'dark', ...defaults }
  }) as AppConfig

describe('registering to open at sign-in', () => {
  it('is off until asked', () => {
    expect(loginItemFor(withDefaults({}), true)).toBe(false)
  })

  it('is on when asked', () => {
    expect(loginItemFor(withDefaults({ startAtLogin: true }), true)).toBe(true)
  })

  it('leaves the record alone before config has loaded', () => {
    // Nothing yet means nothing to say, not "off": unsetting a record the last
    // packaged run wrote would be acting on an answer nobody has given.
    expect(loginItemFor(null, true)).toBeNull()
  })

  it('never touches the record from a development build', () => {
    // A dev build would register the Electron binary under node_modules, and the
    // machine would keep trying to launch it after the checkout was gone. So it
    // must not write "on" -- and must not write "off" either, which would unset
    // what the packaged app had set.
    expect(loginItemFor(withDefaults({ startAtLogin: true }), false)).toBeNull()
    expect(loginItemFor(withDefaults({ startAtLogin: false }), false)).toBeNull()
  })
})
