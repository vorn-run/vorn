import { describe, it, expect } from 'vitest'
import { resolveIntentMode, SHELL_BUILTINS } from '../src/renderer/lib/intent-mode'

const KNOWN = new Set(['git', 'yarn', 'ls', 'node', 'rg', ...SHELL_BUILTINS])

describe('resolveIntentMode', () => {
  it('treats an empty input as shell', () => {
    expect(resolveIntentMode('', KNOWN)).toBe('shell')
    expect(resolveIntentMode('   ', KNOWN)).toBe('shell')
  })

  it('forces shell when the known set is empty', () => {
    // The executable list is absent in older preloads and in tests. Guessing
    // "prompt" there would launch agents on ordinary typing.
    expect(resolveIntentMode('why is this failing', new Set())).toBe('shell')
    expect(resolveIntentMode('add a test for the spine', new Set())).toBe('shell')
  })

  it.each([
    ['git status'],
    ['yarn vitest run tests/x.test.ts'],
    ['ls -la'],
    ['cd packages/server'],
    ['export FOO=1']
  ])('resolves %j as a command', (input) => {
    expect(resolveIntentMode(input, KNOWN)).toBe('shell')
  })

  it('keeps a known command in shell mode even when the rest reads as prose', () => {
    expect(resolveIntentMode('git how do I rebase onto main', KNOWN)).toBe('shell')
  })

  it('treats a lone unknown word as a mistyped command, not a prompt', () => {
    expect(resolveIntentMode('gti', KNOWN)).toBe('shell')
    expect(resolveIntentMode('kubectl', KNOWN)).toBe('shell')
  })

  it.each([
    ['./run.sh --watch'],
    ['../scripts/build.sh now'],
    ['/usr/bin/env node'],
    ['~/bin/deploy staging'],
    ['$EDITOR the file'],
    ['!! and again'],
    ['FOO=bar some command']
  ])('resolves %j as a command on shell syntax', (input) => {
    expect(resolveIntentMode(input, KNOWN)).toBe('shell')
  })

  it.each([
    ['unknowncmd | grep x'],
    ['something && other thing'],
    ['make a thing > out.txt'],
    ['read the $(cat file) output'],
    ['run `date` please'],
    ['do this; then that']
  ])('resolves %j as a command on operators', (input) => {
    expect(resolveIntentMode(input, KNOWN)).toBe('shell')
  })

  it.each([
    ['why did the reflow test start failing'],
    ['rewrite the shim so nested shells keep ZDOTDIR'],
    ['add a test for the spine layout']
  ])('resolves %j as a prompt', (input) => {
    expect(resolveIntentMode(input, KNOWN)).toBe('prompt')
  })

  it('does not treat quoted operators as shell syntax', () => {
    expect(resolveIntentMode('explain the "a || b" idiom to me', KNOWN)).toBe('prompt')
    expect(resolveIntentMode("explain the 'x | y' pipe here", KNOWN)).toBe('prompt')
  })

  it('resolves against the first line only', () => {
    expect(resolveIntentMode('git status\nand then explain it', KNOWN)).toBe('shell')
    expect(resolveIntentMode('explain this please\ngit status', KNOWN)).toBe('prompt')
  })
})
