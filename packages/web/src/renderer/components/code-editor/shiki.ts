/**
 * Syntax colour for code the app shows, from Shiki.
 *
 * One highlighter serves every pane, and it is only loaded the first time
 * something asks for colour, because Shiki is heavy and most views never need
 * it. Languages are loaded on demand for the same reason.
 */

const EXT_TO_LANG: Record<string, string> = {
  ts: 'typescript',
  tsx: 'tsx',
  mts: 'typescript',
  cts: 'typescript',
  js: 'javascript',
  jsx: 'jsx',
  mjs: 'javascript',
  cjs: 'javascript',
  json: 'json',
  jsonc: 'jsonc',
  json5: 'json5',
  html: 'html',
  htm: 'html',
  vue: 'vue',
  svelte: 'svelte',
  css: 'css',
  scss: 'scss',
  sass: 'sass',
  less: 'less',
  md: 'markdown',
  mdx: 'mdx',
  py: 'python',
  pyi: 'python',
  rs: 'rust',
  go: 'go',
  java: 'java',
  kt: 'kotlin',
  swift: 'swift',
  rb: 'ruby',
  php: 'php',
  lua: 'lua',
  zig: 'zig',
  c: 'c',
  h: 'c',
  cpp: 'cpp',
  cc: 'cpp',
  hpp: 'cpp',
  cxx: 'cpp',
  cs: 'csharp',
  sh: 'bash',
  bash: 'bash',
  zsh: 'bash',
  fish: 'fish',
  sql: 'sql',
  graphql: 'graphql',
  gql: 'graphql',
  yml: 'yaml',
  yaml: 'yaml',
  toml: 'toml',
  ini: 'ini',
  xml: 'xml',
  svg: 'xml',
  dockerfile: 'dockerfile',
  makefile: 'makefile',
  r: 'r',
  dart: 'dart',
  ex: 'elixir',
  exs: 'elixir',
  prisma: 'prisma',
  tf: 'hcl',
  ps1: 'powershell',
  bat: 'batch'
}

const FILENAME_TO_LANG: Record<string, string> = {
  dockerfile: 'dockerfile',
  makefile: 'makefile',
  '.gitignore': 'gitignore',
  '.env': 'dotenv'
}

export function getLang(name: string): string | undefined {
  const lower = name.toLowerCase()
  if (FILENAME_TO_LANG[lower]) return FILENAME_TO_LANG[lower]
  const ext = lower.includes('.') ? lower.split('.').pop()! : undefined
  return ext ? EXT_TO_LANG[ext] : undefined
}

export type { TokenLine } from './shiki-core'
import { createTokenizer, type Tokenizer, type TokenLine } from './shiki-core'

/**
 * Highlighting on this thread, with Shiki's JavaScript regex engine: what every
 * pane used before the worker, and what runs where a worker cannot.
 */
let inThread: Tokenizer | null = null
function highlightInThread(code: string, lang: string): Promise<TokenLine[]> {
  inThread ??= createTokenizer(() =>
    import('shiki').then((m) =>
      m.createHighlighter({
        themes: ['vitesse-dark'],
        langs: [],
        engine: m.createJavaScriptRegexEngine()
      })
    )
  )
  return inThread(code, lang)
}

interface Pending {
  code: string
  lang: string
  resolve: (tokens: TokenLine[]) => void
  reject: (err: unknown) => void
}

/**
 * The worker every pane shares, started on the first request.
 *
 * Tokenizing a large file takes tens of milliseconds of regex work, which on
 * this thread is a dropped frame for every terminal on screen. In the worker it
 * runs beside them, with Oniguruma compiled to WASM, the engine the grammars
 * were written for.
 *
 * A worker that cannot start or dies takes nothing with it: everything it was
 * asked and everything asked after goes to this thread instead.
 */
let worker: Worker | null = null
let workerBroken = typeof Worker !== 'function'
let nextId = 0
const pending = new Map<number, Pending>()

function fallBackToThread(): void {
  workerBroken = true
  worker?.terminate()
  worker = null
  const stranded = [...pending.values()]
  pending.clear()
  for (const p of stranded) highlightInThread(p.code, p.lang).then(p.resolve, p.reject)
}

function startWorker(): Worker | null {
  if (worker || workerBroken) return worker
  try {
    worker = new Worker(new URL('./shiki.worker.ts', import.meta.url), { type: 'module' })
  } catch {
    workerBroken = true
    return null
  }
  worker.onmessage = (
    event: MessageEvent<{ id: number; tokens?: TokenLine[]; error?: string }>
  ) => {
    const { id, tokens, error } = event.data
    const p = pending.get(id)
    if (!p) return
    pending.delete(id)
    if (tokens) p.resolve(tokens)
    else p.reject(new Error(error ?? 'highlight failed'))
  }
  worker.onerror = (event) => {
    event.preventDefault?.()
    fallBackToThread()
  }
  return worker
}

export async function highlightCode(code: string, lang: string): Promise<TokenLine[]> {
  const w = startWorker()
  if (!w) return highlightInThread(code, lang)
  return new Promise((resolve, reject) => {
    const id = ++nextId
    pending.set(id, { code, lang, resolve, reject })
    w.postMessage({ id, code, lang })
  })
}
