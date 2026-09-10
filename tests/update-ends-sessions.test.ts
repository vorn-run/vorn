import { describe, it, expect } from 'vitest'
import { updateEndsSessions } from '../src/main/server/handoff-request'

describe('whether an update ends the sessions', () => {
  it('does on Windows, where the installer cannot replace a running exe and nothing hands a terminal over', () => {
    expect(updateEndsSessions('win32', true)).toBe(true)
  })

  it('does not where the next build takes the server over', () => {
    expect(updateEndsSessions('darwin', true)).toBe(false)
    expect(updateEndsSessions('linux', true)).toBe(false)
  })

  it("does not when the server is somebody else's, even on Windows", () => {
    expect(updateEndsSessions('win32', false)).toBe(false)
  })
})
