// @vitest-environment jsdom
import { describe, it, expect } from 'vitest'
import { normalizeUrl, displayHost } from '../src/renderer/lib/browser-url'

/**
 * The address bar has to guess a scheme, because people type `localhost:5173`
 * far more often than a full absolute URL. Guessing wrong is worse than
 * useless: an unrecognised scheme either fails silently in the webview or, for
 * `file:` and `javascript:`, hands the page more reach than a pane should have.
 */
describe('normalizeUrl', () => {
  it('sends bare host:port to http, because that is a dev server', () => {
    // The single most common input for this feature.
    expect(normalizeUrl('localhost:5173')).toBe('http://localhost:5173/')
    expect(normalizeUrl('127.0.0.1:8080/api')).toBe('http://127.0.0.1:8080/api')
    expect(normalizeUrl('myapp.local:3000')).toBe('http://myapp.local:3000/')
  })

  it('recognises a bracketed IPv6 loopback as local', () => {
    // The port can only be split off after the closing bracket — splitting on
    // the first colon yields `[`, which reads as public and picks https, a
    // scheme no local dev server speaks.
    expect(normalizeUrl('[::1]:5173')).toBe('http://[::1]:5173/')
    expect(normalizeUrl('[::1]')).toBe('http://[::1]/')
  })

  it('still treats a routable IPv6 address as public', () => {
    expect(normalizeUrl('[2001:db8::1]:8080')).toBe('https://[2001:db8::1]:8080/')
  })

  it('defaults public hosts to https', () => {
    expect(normalizeUrl('example.com')).toBe('https://example.com/')
    expect(normalizeUrl('example.com/docs?a=1')).toBe('https://example.com/docs?a=1')
    expect(normalizeUrl('//cdn.example.com/x')).toBe('https://cdn.example.com/x')
  })

  it('passes through absolute http(s) urls untouched', () => {
    expect(normalizeUrl('https://x.com/a?b=1')).toBe('https://x.com/a?b=1')
    expect(normalizeUrl('http://localhost:3000')).toBe('http://localhost:3000/')
    expect(normalizeUrl('about:blank')).toBe('about:blank')
  })

  it('refuses schemes a session pane has no business loading', () => {
    // file: would expose the disk, javascript: would execute in the guest.
    expect(normalizeUrl('file:///etc/passwd')).toBeNull()
    expect(normalizeUrl('javascript:alert(1)')).toBeNull()
    expect(normalizeUrl('data:text/html,<h1>x')).toBeNull()
    expect(normalizeUrl('ftp://h.com')).toBeNull()
  })

  it('rejects input that is not a url at all', () => {
    expect(normalizeUrl('')).toBeNull()
    expect(normalizeUrl('   ')).toBeNull()
    expect(normalizeUrl('not a url')).toBeNull()
  })

  it('trims surrounding whitespace from a paste', () => {
    expect(normalizeUrl('  example.com  ')).toBe('https://example.com/')
  })
})

describe('displayHost', () => {
  it('keeps the port only when it distinguishes the target', () => {
    // Pane headers and dock pills are narrow; the host is the useful part.
    expect(displayHost('http://localhost:5173/some/deep/path')).toBe('localhost:5173')
    expect(displayHost('https://example.com/a/b')).toBe('example.com')
  })

  it('falls back to the raw string when there is no host to show', () => {
    // `about:blank` parses cleanly but has an empty hostname — rendering that
    // would leave the pane header blank.
    expect(displayHost('about:blank')).toBe('about:blank')
    expect(displayHost('nonsense')).toBe('nonsense')
  })
})
