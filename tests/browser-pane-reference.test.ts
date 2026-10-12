import fs from 'node:fs'
import path from 'node:path'
import { describe, it, expect, vi } from 'vitest'

vi.mock('electron', () => ({ webContents: { fromId: () => null } }))

import { normalizeUrl, displayHost } from '../src/shared/browser-url'
import { flattenPageText } from '../src/renderer/lib/browser-url'
import { placePopover } from '../src/renderer/lib/artifact-comments'
import { paragraphs, mergeDocEdit } from '../src/renderer/lib/doc-edits'
import { placeArtboards, fitZoom, clampZoom, artboardUrl } from '../src/renderer/lib/design-canvas'
import { mergeTweaks } from '../src/renderer/lib/design-tweaks'
import { bezelFor, steppedZoom } from '../src/renderer/lib/device-bezel'
import { parseManifest } from '../src/main/browser-registry'

/**
 * The browser, artifact and device pane logic the desktop app ports, run
 * over fixed and seeded inputs and recorded as
 * `tests/fixtures/js-reference/browser-pane.json`. The app's Rust checks
 * itself against the same file (`crates/vorn-app/tests/js_reference.rs`).
 *
 * With `VORN_RECORD_JS_REFERENCE=1` the file is written; otherwise this checks
 * the TypeScript still gives what was recorded.
 */
const FILE = path.join(__dirname, 'fixtures', 'js-reference', 'browser-pane.json')
const recording = process.env.VORN_RECORD_JS_REFERENCE === '1'

function seeded(seed: number): () => number {
  let a = seed >>> 0
  return () => {
    a = (a + 0x6d2b79f5) >>> 0
    let t = a
    t = Math.imul(t ^ (t >>> 15), t | 1)
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61)
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296
  }
}

const rand = seeded(0x5eed)
const int = (lo: number, hi: number): number => lo + Math.floor(rand() * (hi - lo + 1))
const pick = <T>(xs: readonly T[]): T => xs[int(0, xs.length - 1)]

const URL_PARTS = [
  'localhost',
  'localhost:3000',
  '127.0.0.1:8080',
  'example.com',
  'https://',
  'http://',
  'file://',
  'about:blank',
  '/path',
  '?q=1',
  '#top',
  ' ',
  'foo',
  'bar.dev',
  ':',
  '%20',
  '[::1]',
  'javascript:',
  'mailto:a@b.c',
  'data:text/html,hi'
] as const

const URL_FIXED = [
  '',
  '   ',
  'localhost:3000',
  'localhost',
  'example.com',
  'example.com/path?x=1#h',
  'http://example.com',
  'https://Example.COM:443/a',
  'file:///tmp/a.html',
  'about:blank',
  'javascript:alert(1)',
  'foo bar',
  '192.168.1.10:5173/app',
  '[::1]:3000',
  'chrome://settings',
  'ftp://example.com'
]

const WORDS = ['alpha', 'beta', 'gamma', 'delta', 'the', 'quick', 'fox', '**bold**', '- item']

function paragraph(): string {
  return Array.from({ length: int(1, 6) }, () => pick(WORDS)).join(' ')
}

function doc(): string[] {
  return Array.from({ length: int(0, 6) }, paragraph)
}

function edited(paras: string[]): string[] {
  const out = [...paras]
  for (let i = int(0, 3); i > 0; i--) {
    const op = int(0, 2)
    const at = int(0, out.length)
    if (op === 0) out.splice(at, 0, paragraph())
    else if (op === 1 && out.length) out.splice(Math.min(at, out.length - 1), 1)
    else if (out.length) out[Math.min(at, out.length - 1)] = paragraph()
  }
  return out
}

function boards(): { id: string; label: string; width: number; height: number }[] {
  return Array.from({ length: int(0, 4) }, (_, i) => ({
    id: `b${i}`,
    label: `Board ${i}`,
    width: int(120, 1600),
    height: int(120, 1200)
  }))
}

const TWEAK_MANIFEST = JSON.stringify({
  kind: 'design',
  title: 'Tweaks',
  tweaks: {
    size: { type: 'number', default: 12, min: 8, max: 40, step: 1, unit: 'px' },
    dark: { type: 'boolean', default: false },
    accent: { type: 'color', default: '#ff0000' },
    tone: { type: 'color', default: '#000', options: ['#000', '#fff'] },
    plan: { type: 'select', default: 'free', options: ['free', 'pro'] }
  },
  artboards: [{ id: 'a', label: 'A', width: 390, height: 844 }]
})

const TWEAK_VALUES = [12, 0, -3.5, true, false, '#00ff00', '#fff', 'pro', 'gone', null] as const

function record(): Record<string, unknown> {
  const urls = [
    ...URL_FIXED,
    ...Array.from({ length: 80 }, () =>
      Array.from({ length: int(1, 4) }, () => pick(URL_PARTS)).join('')
    )
  ]
  const normalize = urls.flatMap((u) =>
    [false, true].map((allowFile) => ({
      input: u,
      allowFile,
      out: normalizeUrl(u, { allowFile })
    }))
  )
  const hosts = normalize
    .map((c) => c.out)
    .filter((u): u is string => u !== null)
    .concat(['not a url', 'file:///Users/me/My%20Site/index.html', 'about:blank'])
    .map((u) => ({ input: u, out: displayHost(u) }))
  const flatten = Array.from({ length: 30 }, () => {
    const raw = Array.from({ length: int(0, 40) }, () =>
      pick([...WORDS, '\n', '\t', '  ', '\n\n'])
    ).join(pick([' ', '', '\n']))
    const max = int(5, 120)
    return { raw, max, out: flattenPageText(raw, max) }
  })
  const docs = Array.from({ length: 60 }, () => {
    const before = doc()
    const sep = pick(['\n\n', '\n\n\n', '\n  \n'])
    const original = before.join(sep)
    const edit = edited(before).join('\n\n')
    return {
      original,
      edited: edit,
      paragraphs: paragraphs(original),
      merged: mergeDocEdit(original, edit)
    }
  })
  const canvas = Array.from({ length: 40 }, () => {
    const b = boards()
    const area = { width: int(-10, 2400), height: int(-10, 1600) }
    const zoom = fitZoom(b, area)
    const placed = placeArtboards(b, zoom)
    return {
      boards: b,
      area,
      zoom,
      placed: placed.placed.map((p) => ({ left: p.left, top: p.top })),
      width: placed.width,
      height: placed.height
    }
  })
  const clamp = Array.from({ length: 30 }, () => {
    const z = rand() * 2 - 0.25
    return { input: z, out: clampZoom(z) }
  })
  const artboardUrls = [
    ['file:///a/b.html', 'main'],
    ['file:///a/b.html#old', 'x y'],
    ['http://localhost:3000/#/route', 'é/ü'],
    ['http://h/p?q=1', "a-b_c.d!~*'()"]
  ].map(([url, id]) => ({ url, id, out: artboardUrl(url, id) }))
  const popover = Array.from({ length: 40 }, () => {
    const rect = { x: int(-20, 900), y: int(-20, 700), width: int(0, 300), height: int(0, 80) }
    const area = { width: int(200, 1000), height: int(150, 800) }
    return { rect, area, out: placePopover(rect, area) }
  })
  const bezel = Array.from({ length: 60 }, () => {
    const screen =
      rand() < 0.1
        ? { width: 0, height: int(0, 900) }
        : pick([
            { width: 393, height: 852 },
            { width: 430, height: 932 },
            { width: 1024, height: 1366 },
            { width: 834, height: 1194 },
            { width: 852, height: 393 },
            { width: int(200, 1400), height: int(200, 1400) }
          ])
    const container = { width: int(0, 1400), height: int(0, 1200) }
    const b = bezelFor(screen, container, 'fit', null)
    return {
      screen,
      container,
      scale: b.scale,
      width: b.width,
      height: b.height,
      thickness: b.inset.left,
      outerRadius: b.outerRadius
    }
  })
  const zoom = Array.from({ length: 40 }, () => {
    const current = Number((rand() * 4.5).toFixed(3))
    const direction = pick([1, -1] as const)
    return { current, direction, out: steppedZoom(current, direction) }
  })
  const declared = parseManifest(TWEAK_MANIFEST)?.tweaks
  const tweaks = Array.from({ length: 40 }, () => {
    const set = (): Record<string, unknown> =>
      Object.fromEntries(
        ['size', 'dark', 'accent', 'tone', 'plan', 'other']
          .filter(() => rand() < 0.5)
          .map((k) => [k, pick(TWEAK_VALUES)])
      )
    const stored = set()
    const live = rand() < 0.3 ? undefined : set()
    return {
      stored,
      live: live ?? null,
      out: mergeTweaks(declared, stored as never, live)
    }
  })
  const manifests = [
    '',
    'not json',
    '[]',
    '{"kind":"other"}',
    '{"kind":"page"}',
    '{"kind":"doc","title":"  Notes  "}',
    TWEAK_MANIFEST,
    JSON.stringify({ kind: 'page', title: 'x'.repeat(200) }),
    JSON.stringify({
      kind: 'design',
      artboards: [
        { id: 'a', label: 'A', width: 100, height: 100 },
        { id: 'a', label: 'dup', width: 100, height: 100 },
        { id: 'bad id', label: 'B', width: 100, height: 100 },
        { id: 'c', label: 'C', width: -1, height: 100 }
      ]
    }),
    JSON.stringify({
      kind: 'page',
      tweaks: {
        '1bad': { type: 'number', default: 1 },
        ok: { type: 'select', default: 'z', options: ['a'] },
        n: { type: 'number', default: 'x' }
      }
    })
  ].map((text) => ({ text, out: parseManifest(text) }))
  return {
    normalizeUrl: normalize,
    displayHost: hosts,
    flattenPageText: flatten,
    docEdits: docs,
    canvas,
    clampZoom: clamp,
    artboardUrl: artboardUrls,
    placePopover: popover,
    bezel,
    steppedZoom: zoom,
    tweakManifest: TWEAK_MANIFEST,
    mergeTweaks: tweaks,
    parseManifest: manifests
  }
}

describe('browser pane logic against its JS reference', () => {
  it('gives what was recorded', () => {
    const live = JSON.parse(JSON.stringify(record()))
    if (recording) {
      fs.writeFileSync(FILE, JSON.stringify(live, null, 2) + '\n')
      return
    }
    const recorded = JSON.parse(fs.readFileSync(FILE, 'utf-8'))
    expect(live).toEqual(recorded)
  })
})
