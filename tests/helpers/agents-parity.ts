/**
 * What may differ between the server's answer to an `agent:`, `sessions:` or
 * `shell:` call and vornd's own answer to it, and nothing else.
 *
 * Answers are compared as `helpers/git-parity`'s `answerOf` reads them: the
 * frame without its id. Each normalizer below names one accepted difference.
 *
 * - {@link catalogFetchedAt}: a model list records when its CLI answered,
 *   and each side asks its own CLI at its own moment.
 */
import type { Answer } from './git-parity'

/** An `agent:listModels` answer's `fetchedAt` is `<fetched>`. */
export function catalogFetchedAt(answer: Answer): Answer {
  const result = answer.result as { fetchedAt?: unknown } | undefined
  if (!result || typeof result !== 'object' || typeof result.fetchedAt !== 'number') return answer
  return { ...answer, result: { ...result, fetchedAt: '<fetched>' } }
}
