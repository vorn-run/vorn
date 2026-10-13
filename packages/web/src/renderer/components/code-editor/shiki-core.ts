/**
 * Shiki's tokens for some code, the same in the worker and on the main thread.
 */
export type TokenLine = { content: string; color?: string }[]

type Highlighter = Awaited<ReturnType<typeof import('shiki').createHighlighter>>

export type Tokenizer = (code: string, lang: string) => Promise<TokenLine[]>

/**
 * A tokenizer over one highlighter, made the first time it is needed by `load`,
 * that loads each language the first time it is asked for. A language Shiki
 * does not know gives no tokens rather than an error.
 */
export function createTokenizer(load: () => Promise<Highlighter>): Tokenizer {
  let highlighter: Promise<Highlighter> | null = null
  const loaded = new Set<string>()
  return async (code, lang) => {
    highlighter ??= load()
    const hl = await highlighter
    if (!loaded.has(lang)) {
      try {
        await hl.loadLanguage(lang as Parameters<typeof hl.loadLanguage>[0])
        loaded.add(lang)
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
}
