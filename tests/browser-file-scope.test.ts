import { describe, it, expect, beforeEach, afterAll } from 'vitest'
import { mkdtempSync, mkdirSync, writeFileSync, symlinkSync, rmSync, realpathSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { pathToFileURL } from 'node:url'

import {
  setFileRoot,
  hasFileRoot,
  resetFileRoots,
  containsPath,
  allowsFileUrl
} from '../src/main/browser-file-scope'

/**
 * The boundary around a pane's reach into the disk.
 *
 * Every case here is one where a refusal and an allow look alike from the
 * outside — a path that is textually inside the root, a link that is inside it
 * and points out, an escape spelled in percent-encoding. The pane is driven by
 * an agent reading pages it did not choose, so "looks contained" is not the
 * standard; resolved-and-contained is.
 */

// A real directory tree: symlink and traversal behaviour is the thing under
// test, and neither can be faked convincingly.
const tmp = realpathSync(mkdtempSync(join(tmpdir(), 'vorn-scope-')))
const project = join(tmp, 'project')
const outside = join(tmp, 'outside')

mkdirSync(project)
mkdirSync(outside)
mkdirSync(join(project, 'src'))
writeFileSync(join(project, 'index.html'), '<h1>ok</h1>')
writeFileSync(join(project, 'src', 'page.html'), 'nested')
writeFileSync(join(outside, 'secret.txt'), 'do not read me')
// The classic containment defeat: inside the root by name, anywhere by target.
symlinkSync(outside, join(project, 'escape'))
// A sibling sharing the root's prefix, which a naive startsWith would swallow.
const sibling = `${project}-secret`
mkdirSync(sibling)
writeFileSync(join(sibling, 'creds.txt'), 'nope')

afterAll(() => rmSync(tmp, { recursive: true, force: true }))

const url = (p: string): string => pathToFileURL(p).href

beforeEach(() => {
  resetFileRoots()
  setFileRoot('sess', project)
})

describe('what counts as inside the root', () => {
  it('allows the root itself and files under it', () => {
    expect(containsPath(project, project)).toBe(true)
    expect(containsPath(project, join(project, 'index.html'))).toBe(true)
    expect(containsPath(project, join(project, 'src', 'page.html'))).toBe(true)
  })

  it('refuses a sibling that merely shares the root’s name as a prefix', () => {
    // `/proj` must not contain `/proj-secret`. They share a prefix and nothing
    // else, and a plain startsWith would hand over the second.
    expect(containsPath(project, join(sibling, 'creds.txt'))).toBe(false)
  })

  it('refuses a path that climbs out with ..', () => {
    expect(containsPath(project, join(project, '..', 'outside', 'secret.txt'))).toBe(false)
  })

  it('refuses a symlink that sits inside the root and points out of it', () => {
    // Textually this is inside the project. Following it is the only way to
    // find out it is not, which is why both sides are resolved.
    expect(containsPath(project, join(project, 'escape', 'secret.txt'))).toBe(false)
  })

  it('refuses a path that does not exist, rather than guessing at it', () => {
    // Unresolvable means unprovable. There is nothing to render either way, so
    // the safe answer costs nothing.
    expect(containsPath(project, join(project, 'no-such-file.html'))).toBe(false)
  })
})

describe('file urls a session may open', () => {
  it('allows a file inside the session’s own root', () => {
    expect(allowsFileUrl('sess', url(join(project, 'index.html')))).toBe(true)
  })

  it('refuses everything when the session has no root at all', () => {
    // A session that never reported one — a headless run, or a pane belonging
    // to no project. Absent must mean nothing, never everything.
    setFileRoot('sess', undefined)
    expect(allowsFileUrl('sess', url(join(project, 'index.html')))).toBe(false)
  })

  it('refuses another session’s root', () => {
    setFileRoot('other', outside)
    expect(allowsFileUrl('sess', url(join(outside, 'secret.txt')))).toBe(false)
    expect(allowsFileUrl('other', url(join(outside, 'secret.txt')))).toBe(true)
  })

  it('refuses an escape spelled in percent-encoding', () => {
    // The guest decodes this before it opens anything, so the check has to see
    // the same path the filesystem will.
    const encoded = `file://${project}/..%2Foutside%2Fsecret.txt`
    expect(allowsFileUrl('sess', encoded)).toBe(false)
  })

  it('treats a localhost host the way the url spec does, and still bounds it', () => {
    // WHATWG erases `localhost` from a file url, so this parses with an empty
    // host and is an ordinary local path — the same answer `normalizeUrl`
    // gives. What keeps it honest is the root, not the host: in-root passes,
    // out-of-root does not.
    expect(allowsFileUrl('sess', `file://localhost${join(project, 'index.html')}`)).toBe(true)
    expect(allowsFileUrl('sess', `file://localhost${join(outside, 'secret.txt')}`)).toBe(false)
  })

  it('refuses a UNC url naming another machine', () => {
    // `file://host/share` is not a local file, and reading it as one would
    // reach the network from a check that only ever thought about disks.
    expect(allowsFileUrl('sess', 'file://evil.example/share/secret.txt')).toBe(false)
  })

  it('refuses a url that is not a file url', () => {
    expect(allowsFileUrl('sess', 'https://example.com/')).toBe(false)
  })

  it('stores a root only if it resolves, so a bogus one grants nothing', () => {
    // An unresolvable root kept as a plain string could still prefix-match a
    // path that does exist. Dropping it means the session simply has no reach.
    setFileRoot('ghost', join(tmp, 'not-a-real-dir'))
    expect(hasFileRoot('ghost')).toBe(false)
    expect(allowsFileUrl('ghost', url(join(project, 'index.html')))).toBe(false)
  })

  it('records a root through the link that led to it', () => {
    // The root is resolved on the way in, so a session pointed at a symlinked
    // checkout still matches the paths its guest will actually report.
    const linked = join(tmp, 'linked-project')
    symlinkSync(project, linked)
    setFileRoot('via-link', linked)
    expect(hasFileRoot('via-link')).toBe(true)
    expect(allowsFileUrl('via-link', url(join(project, 'index.html')))).toBe(true)
  })
})
