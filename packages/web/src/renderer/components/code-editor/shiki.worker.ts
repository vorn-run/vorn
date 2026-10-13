/**
 * Shiki off the main thread; see `highlightCode` in `shiki.ts`.
 *
 * Oniguruma compiled to WASM is the engine the TextMate grammars were written
 * for. If it cannot start here (a policy that forbids compiling WASM, say),
 * the JavaScript regex engine the main thread uses takes over, so the worker
 * still answers.
 */
import { createTokenizer } from './shiki-core'

const tokenize = createTokenizer(async () => {
  const shiki = await import('shiki')
  const options = { themes: ['vitesse-dark'], langs: [] }
  try {
    const { createOnigurumaEngine } = await import('shiki/engine/oniguruma')
    return await shiki.createHighlighter({
      ...options,
      engine: createOnigurumaEngine(import('shiki/wasm'))
    })
  } catch {
    return shiki.createHighlighter({ ...options, engine: shiki.createJavaScriptRegexEngine() })
  }
})

interface Request {
  id: number
  code: string
  lang: string
}

const scope = self as unknown as {
  onmessage: ((event: MessageEvent<Request>) => void) | null
  postMessage(message: unknown): void
}

scope.onmessage = (event) => {
  const { id, code, lang } = event.data
  tokenize(code, lang).then(
    (tokens) => scope.postMessage({ id, tokens }),
    (err: unknown) =>
      scope.postMessage({ id, error: err instanceof Error ? err.message : String(err) })
  )
}
