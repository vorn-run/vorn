import { describe, it, expect } from 'vitest'
import { parseLaunchSpec } from '../src/renderer/components/settings/parse-launch-spec'

describe('parseLaunchSpec', () => {
  it('runs a bare package name through npx so nothing has to be installed first', () => {
    expect(parseLaunchSpec('@vornrun/connector-kusto')).toEqual({
      command: 'npx',
      args: ['-y', '@vornrun/connector-kusto']
    })
  })

  it('ignores surrounding whitespace from a pasted package name', () => {
    expect(parseLaunchSpec('  my-connector  ')).toEqual({
      command: 'npx',
      args: ['-y', 'my-connector']
    })
  })

  it('keeps a versioned package name intact', () => {
    expect(parseLaunchSpec('@vornrun/connector-kusto@0.2.0')).toEqual({
      command: 'npx',
      args: ['-y', '@vornrun/connector-kusto@0.2.0']
    })
  })

  it('runs a multi-word command as written, for a connector under development', () => {
    expect(parseLaunchSpec('node ./dist/index.js --verbose')).toEqual({
      command: 'node',
      args: ['./dist/index.js', '--verbose']
    })
  })

  it('treats a lone path as an executable rather than a package to download', () => {
    expect(parseLaunchSpec('./dist/index.js')).toEqual({ command: './dist/index.js', args: [] })
    expect(parseLaunchSpec('~/bin/connector')).toEqual({ command: '~/bin/connector', args: [] })
    expect(parseLaunchSpec('/usr/local/bin/connector')).toEqual({
      command: '/usr/local/bin/connector',
      args: []
    })
  })
})
