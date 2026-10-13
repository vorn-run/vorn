import type { Outline } from '../completions'
import names from './names.json'

/**
 * Generated completion index.
 *
 * Command names come from names.json, which is small and loaded eagerly.
 * Outlines are one file per command, imported on demand — completing `git`
 * must not pull in every cloud CLI in the index.
 *
 * Regenerate with `yarn gen:completions`. Attribution for the upstream
 * completion-spec corpus is in ./LICENSE and ./meta.json.
 */

export interface OutlineSource {
  /** Command name to one-line description. */
  names(): Promise<Map<string, string | undefined>>
  outline(name: string): Promise<Outline | undefined>
}

const outlineLoaders = import.meta.glob<{ default: Outline }>('./outlines/*.json')

const outlineCache = new Map<string, Promise<Outline | undefined>>()
let namesCache: Map<string, string | undefined> | null = null

/** The generated index. Missing entries resolve to undefined, never throw. */
export function generatedOutlineSource(): OutlineSource {
  return {
    names: async () => {
      if (!namesCache) {
        // Descriptions are stored as null when absent so the key survives
        // JSON serialisation; the engine wants undefined.
        namesCache = new Map(
          Object.entries(names as Record<string, string | null>).map(
            ([name, detail]) => [name, detail ?? undefined] as const
          )
        )
      }
      return namesCache
    },
    outline: (name) => {
      const cached = outlineCache.get(name)
      if (cached) return cached
      const load = outlineLoaders[`./outlines/${name}.json`]
      const promise: Promise<Outline | undefined> = load
        ? load()
            .then((m) => m.default)
            .catch(() => undefined)
        : Promise.resolve(undefined)
      outlineCache.set(name, promise)
      return promise
    }
  }
}

/** In-memory source for tests and for the hand-written outlines. */
export function staticOutlineSource(outlines: Record<string, Outline>): OutlineSource {
  return {
    names: async () =>
      new Map(Object.entries(outlines).map(([name, o]) => [name, o.detail] as const)),
    outline: async (name) => outlines[name]
  }
}
