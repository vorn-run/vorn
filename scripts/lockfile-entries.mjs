// Lockfile entry regrouping, kept apart from check-lockfile.mjs so it can be
// tested without that script's file I/O and process.exit.

/**
 * Split an entry header into its descriptors.
 *
 * Yarn quotes the whole comma-separated list as one string rather than each
 * descriptor, so the quotes come off before splitting and go back on once.
 */
export const splitDescriptors = (header) => {
  const unquoted = header.startsWith('"') && header.endsWith('"') ? header.slice(1, -1) : header
  // Not trimmed: a range may legitimately end in a space, and altering the
  // descriptor makes the package unresolvable.
  return unquoted.split(', ')
}

export const joinDescriptors = (descriptors) => {
  const list = [...new Set(descriptors)]
    // Yarn writes descriptors in plain lexicographic order of the raw string.
    .sort((a, b) => (a < b ? -1 : a > b ? 1 : 0))
    .join(', ')
  // Yarn quotes a header containing a character YAML would otherwise read as
  // structure, which every `name@npm:range` descriptor does.
  return /[:,]/.test(list) ? `"${list}"` : list
}

/**
 * Re-merge entries that only differed by their archive URL.
 *
 * Yarn groups descriptors that share a locator into one entry. When a mirror
 * pins an archive URL, two descriptors for the same package version are
 * distinct locators, so Yarn writes them as separate entries — and stripping
 * the URL afterwards cannot undo that split. The lockfile then differs from
 * one resolved against public npm, which fails `yarn install --immutable` in
 * CI with a diff that looks unrelated to the dependency actually being added.
 */
export const mergeDuplicateEntries = (lockfile) => {
  const byBody = new Map()
  const order = []

  for (const block of lockfile.split('\n\n')) {
    const match = /^("(?:[^"]|"")+"|[^\s#][^\n]*):\n([\s\S]*)$/.exec(block)
    // Comments and the metadata preamble have no descriptor header.
    if (!match || !/^\s+(version|resolution):/m.test(match[2])) {
      order.push({ raw: block })
      continue
    }
    const [, header, body] = match
    const existing = byBody.get(body)
    if (existing) {
      existing.descriptors.push(...splitDescriptors(header))
      continue
    }
    const entry = { descriptors: splitDescriptors(header), body }
    byBody.set(body, entry)
    order.push(entry)
  }

  return order
    .map((entry) =>
      entry.raw !== undefined ? entry.raw : `${joinDescriptors(entry.descriptors)}:\n${entry.body}`
    )
    .join('\n\n')
}
