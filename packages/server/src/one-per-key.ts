/**
 * Calls that share a key while one is still running share its result.
 *
 * For work that must not happen twice for the same thing at once, where the
 * second caller wants exactly what the first is producing: two creates for the
 * same conversation want the one session, not two workspaces of which one is
 * discarded.
 */
export function onePerKey<T>(): (key: string, run: () => Promise<T>) => Promise<T> {
  const running = new Map<string, Promise<T>>()
  return (key, run) => {
    const inFlight = running.get(key)
    if (inFlight) return inFlight
    const started = run().finally(() => running.delete(key))
    running.set(key, started)
    return started
  }
}
