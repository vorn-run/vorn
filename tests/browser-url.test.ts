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

  it('keeps refusing file: for anyone who did not ask for it', () => {
    // The address bar and every existing caller pass no options, so opening
    // the capability up cannot change what a person can type.
    expect(normalizeUrl('file:///etc/passwd', {})).toBeNull()
    expect(normalizeUrl('file:///etc/passwd', { allowFile: false })).toBeNull()
  })

  it('lets a caller that asked for it form a file url, and nothing else', () => {
    // Well-formed only. Whether the path is inside the session's root is a
    // filesystem question, answered in main — this says the shape is a file.
    expect(normalizeUrl('file:///proj/index.html', { allowFile: true })).toBe(
      'file:///proj/index.html'
    )
    // The permission is for file:, not a general amnesty.
    expect(normalizeUrl('javascript:alert(1)', { allowFile: true })).toBeNull()
    expect(normalizeUrl('data:text/html,<h1>x', { allowFile: true })).toBeNull()
  })

  it('refuses a file url naming another machine even when file: is allowed', () => {
    // `file://host/share` is a UNC path: it reads as a local file and reaches
    // the network. An empty hostname is what makes a file url local.
    expect(normalizeUrl('file://evil.example/share/secret', { allowFile: true })).toBeNull()
  })

  it('drops a query and fragment from a file url', () => {
    // Neither can address a file, and carrying them would hand the containment
    // check a string that is not the path actually read.
    expect(normalizeUrl('file:///proj/a.html?x=1#frag', { allowFile: true })).toBe(
      'file:///proj/a.html'
    )
  })

  it('still refuses a bare host that merely looks like the file scheme', () => {
    // `file:1234` is shape-identical to `myhost:1234`; the host:port fast path
    // must not turn a refusal into `https://file:1234/`.
    expect(normalizeUrl('file:1234', { allowFile: true })).toBeNull()
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

  it('names a local file by its filename, not its whole path', () => {
    // A tab is a few characters wide. The leading directories push the only
    // part that identifies the file off the end, so a design in a repo showed
    // as an unreadable run of path with the name invisible.
    expect(displayHost('file:///Users/j/dev/vorn/design/true-black.dc.html')).toBe(
      'true-black.dc.html'
    )
    expect(displayHost('file:///repo/a%20b.dc.html')).toBe('a b.dc.html')
  })

  it('keeps the filename when its escaping is malformed', () => {
    // A stray `%` is a legal filename character and an invalid escape, so
    // decoding throws. Falling through to the outer handler would return the
    // whole url — the unreadable label this fix exists to remove.
    expect(displayHost('file:///repo/%ZZ.dc.html')).toBe('%ZZ.dc.html')
  })

  it('names a blank page "New tab" rather than showing the scheme', () => {
    // `about:blank` parses cleanly but has an empty hostname, and the raw
    // string is jargon — a tab you have not navigated yet should read as a
    // place to type, the way every browser labels it.
    expect(displayHost('about:blank')).toBe('New tab')
  })

  it('falls back to the raw string when there is no host to show', () => {
    // Anything else unparseable still beats rendering a blank header.
    expect(displayHost('nonsense')).toBe('nonsense')
  })
})
