import { describe, it, expect } from 'vitest'
import fs from 'node:fs'
import path from 'node:path'
import { PHONE_BASE_TOPICS, topicsQuery, terminalTopic } from '../packages/shared/src/topics'

/**
 * A phone that asks for the wrong namespace goes quiet for that feature, with
 * nothing on screen to say why. So the list is checked against the handlers
 * the web client actually registers, read from the source.
 */
const shim = fs.readFileSync(path.join(__dirname, '../packages/web/src/api-shim.ts'), 'utf8')
const registered = [...shim.matchAll(/rpc\.on\('([a-z]+:[a-zA-Z-]+)'/g)].map((m) => m[1])

function covered(name: string): boolean {
  return PHONE_BASE_TOPICS.some((t) =>
    t.endsWith('*') ? name.startsWith(t.slice(0, -1)) : t === name
  )
}

describe("the phone's base topics", () => {
  it('cover every notification the web client handles, except terminal bytes', () => {
    expect(registered.length).toBeGreaterThan(5)
    const missing = registered.filter((name) => name !== 'terminal:data' && !covered(name))
    expect(missing).toEqual([])
  })

  it('leave terminal bytes to be asked for one card at a time', () => {
    expect(covered('terminal:data')).toBe(false)
    expect(terminalTopic('abc')).toBe('terminal:data#abc')
  })

  it('travel on the socket URL in the form the server parses', () => {
    // `parseTopics` splits on commas; the encoding must survive a round trip.
    const query = topicsQuery(['session:*', 'terminal:data#x'])
    expect(decodeURIComponent(query.slice('topics='.length)).split(',')).toEqual([
      'session:*',
      'terminal:data#x'
    ])
  })
})
