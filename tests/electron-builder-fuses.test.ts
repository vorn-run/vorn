import { describe, it, expect } from 'vitest'
import { readFileSync } from 'node:fs'
import path from 'node:path'

/** The keys set under `electronFuses:`, read as text so the test needs no YAML parser. */
function fuseSettings(): Record<string, string> {
  const lines = readFileSync(path.resolve(__dirname, '../electron-builder.yml'), 'utf8').split('\n')
  const start = lines.findIndex((line) => line.trim() === 'electronFuses:')
  if (start === -1) return {}
  const settings: Record<string, string> = {}
  for (const line of lines.slice(start + 1)) {
    if (!/^\s/.test(line) && line.trim()) break
    const setting = line.match(/^\s+([A-Za-z]+):\s*(\S+)/)
    if (setting) settings[setting[1]!] = setting[2]!
  }
  return settings
}

describe('the fuses the packaged app is built with', () => {
  it('encrypts the cookie store, which cannot be turned off again without corrupting it', () => {
    expect(fuseSettings().enableCookieEncryption).toBe('true')
  })

  it('leaves every other fuse alone, since the server runs through ELECTRON_RUN_AS_NODE', () => {
    expect(Object.keys(fuseSettings())).toEqual(['enableCookieEncryption'])
  })
})
