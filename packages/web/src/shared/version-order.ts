/** Enough to answer "is there something newer", prereleases included. */
export function isNewerVersion(candidate: string, current: string): boolean {
  // Build metadata carries no precedence, so it is dropped before anything is compared.
  const split = (version: string): { release: number[]; pre: Array<number | string> } => {
    const [release, ...pre] = version.split('+')[0].split('-')
    return {
      release: release.split('.').map((part) => (/^\d+$/.test(part) ? Number(part) : 0)),
      pre: pre
        .join('-')
        .split('.')
        .filter((part) => part !== '')
        .map((part) => (/^\d+$/.test(part) ? Number(part) : part))
    }
  }

  const left = split(candidate)
  const right = split(current)
  // A missing release segment is a zero, so 1.0 and 1.0.0 are the same version.
  for (let index = 0; index < Math.max(left.release.length, right.release.length); index++) {
    const a = left.release[index] ?? 0
    const b = right.release[index] ?? 0
    if (a !== b) return a > b
  }
  // Same release: a prerelease loses to it, and to a prerelease that sorts later.
  if (left.pre.length === 0) return right.pre.length > 0
  if (right.pre.length === 0) return false
  for (let index = 0; index < Math.max(left.pre.length, right.pre.length); index++) {
    const a = left.pre[index]
    const b = right.pre[index]
    if (a === b) continue
    if (a === undefined) return false
    if (b === undefined) return true
    if (typeof a === 'number' && typeof b === 'number') return a > b
    // A numeric identifier ranks below an alphanumeric one.
    if (typeof a === 'number') return false
    if (typeof b === 'number') return true
    return a > b
  }
  return false
}
