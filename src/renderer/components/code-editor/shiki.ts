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

export type TokenLine = { content: string; color?: string }[]

type Highlighter = Awaited<ReturnType<typeof import('shiki').createHighlighter>>
let highlighterPromise: Promise<Highlighter> | null = null
const loadedLangs = new Set<string>()

export function getHighlighter(): Promise<Highlighter> {
  if (!highlighterPromise) {
    highlighterPromise = import('shiki').then((m) =>
      m.createHighlighter({
        themes: ['vitesse-dark'],
        langs: [],
        engine: m.createJavaScriptRegexEngine()
      })
    )
  }
  return highlighterPromise
}

export async function highlightCode(code: string, lang: string): Promise<TokenLine[]> {
  const hl = await getHighlighter()
  if (!loadedLangs.has(lang)) {
    try {
      await hl.loadLanguage(lang as Parameters<typeof hl.loadLanguage>[0])
      loadedLangs.add(lang)
    } catch {
      return []
    }
  }
  const result = hl.codeToTokens(code, {
    lang: lang as Parameters<typeof hl.codeToTokens>[1]['lang'],
    theme: 'vitesse-dark'
  })
  return result.tokens.map((line) => line.map((t) => ({ content: t.content, color: t.color })))
}
