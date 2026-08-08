import { describe, it, expect } from 'vitest'
import { formatBytes } from '../src/renderer/lib/format-bytes'

describe('formatBytes', () => {
  it('reports nothing and negatives as zero', () => {
    expect(formatBytes(0)).toBe('0 B')
    expect(formatBytes(-1)).toBe('0 B')
    expect(formatBytes(NaN)).toBe('0 B')
  })

  it('keeps bytes and kilobytes whole', () => {
    expect(formatBytes(512)).toBe('512 B')
    expect(formatBytes(1024)).toBe('1 KB')
    expect(formatBytes(1536)).toBe('2 KB')
  })

  it('keeps one decimal below ten so 1.9 GB does not read as 2 GB', () => {
    expect(formatBytes(1.9 * 1024 ** 3)).toBe('1.9 GB')
    expect(formatBytes(983 * 1024 ** 2)).toBe('983 MB')
  })

  it('drops the decimal at ten and above', () => {
    expect(formatBytes(15 * 1024 ** 2)).toBe('15 MB')
    expect(formatBytes(12 * 1024 ** 3)).toBe('12 GB')
  })

  it('uses binary units, matching what du -h prints', () => {
    // 1_000_000 bytes is "1 MB" only in decimal units; du calls it 976K.
    expect(formatBytes(1_000_000)).toBe('977 KB')
  })
})
