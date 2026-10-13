import { useEffect, useState } from 'react'
import { highlightCode, type TokenLine } from './shiki'

/**
 * The lines of `text` in colour, or null while they are on their way or when
 * `lang` is not one Shiki knows.
 *
 * `loose` keeps the last tokens for the same language while newer ones are on
 * their way, so an editor does not flash to grey on every keystroke; the
 * caller must check each line still matches what it draws.
 */
export function useHighlightedLines(
  text: string,
  lang: string | undefined,
  loose = false
): TokenLine[] | null {
  const [result, setResult] = useState<{
    text: string
    lang: string
    tokens: TokenLine[]
  } | null>(null)

  useEffect(() => {
    if (!lang) return

    let stale = false
    highlightCode(text, lang)
      .then((tokens) => {
        if (stale) return
        setResult(tokens.length > 0 ? { text, lang, tokens } : null)
      })
      .catch(() => {
        if (!stale) setResult(null)
      })

    return () => {
      stale = true
    }
  }, [text, lang])

  if (!lang || !result) return null
  if (result.lang !== lang) return null
  if (!loose && result.text !== text) return null
  return result.tokens
}
