// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'

const platform = { isWeb: true }
vi.mock('../src/renderer/lib/platform', () => ({
  get isWeb() {
    return platform.isWeb
  },
  get isElectron() {
    return !platform.isWeb
  },
  isMac: true,
  MOD: 'Cmd'
}))

import { markVisible, markHidden, resetVisible } from '../src/renderer/lib/visible-terminals'
import { PHONE_BASE_TOPICS } from '../packages/shared/src/topics'

const setTopics = vi.fn(async (_topics: readonly string[]) => {})
Object.defineProperty(window, 'api', { value: { setTopics }, writable: true })

beforeEach(() => {
  vi.useFakeTimers()
  resetVisible()
  setTopics.mockClear()
  platform.isWeb = true
})
afterEach(() => vi.useRealTimers())

const lastTopics = (): readonly string[] => setTopics.mock.calls.at(-1)![0]

describe('what the web client asks the server for', () => {
  it('asks for the base namespaces plus each card on screen, by instance', () => {
    markVisible('a')
    markVisible('b')
    vi.runAllTimers()
    expect(lastTopics()).toEqual([...PHONE_BASE_TOPICS, 'terminal:data#a', 'terminal:data#b'])
  })

  it('folds a scroll into one message', () => {
    markVisible('a')
    markVisible('b')
    markHidden('a')
    markVisible('c')
    vi.runAllTimers()
    expect(setTopics).toHaveBeenCalledTimes(1)
    expect(lastTopics()).toEqual([...PHONE_BASE_TOPICS, 'terminal:data#b', 'terminal:data#c'])
  })

  it('drops a card that left the screen', () => {
    markVisible('a')
    vi.runAllTimers()
    markHidden('a')
    vi.runAllTimers()
    expect(lastTopics()).toEqual([...PHONE_BASE_TOPICS])
  })

  it('never asks for terminal bytes by name', () => {
    markVisible('a')
    vi.runAllTimers()
    expect(lastTopics()).not.toContain('terminal:data')
    expect(lastTopics()).not.toContain('terminal:*')
  })

  it('says nothing when nothing changed', () => {
    markVisible('a')
    vi.runAllTimers()
    markVisible('a')
    markHidden('never-shown')
    vi.runAllTimers()
    expect(setTopics).toHaveBeenCalledTimes(1)
  })

  it('sends nothing from the desktop, which is sent everything', () => {
    platform.isWeb = false
    markVisible('a')
    vi.runAllTimers()
    expect(setTopics).not.toHaveBeenCalled()
  })
})
