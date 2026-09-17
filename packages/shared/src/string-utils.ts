/**
 * Derive a display name from a prompt string.
 * Strips leading whitespace/newlines, truncates to ~60 chars at a word
 * boundary, and appends an ellipsis when truncated.
 * Returns undefined for empty/whitespace-only input.
 */
export function displayNameFromPrompt(prompt: string, maxLen = 60): string | undefined {
  const cleaned = prompt.replace(/^\s+/, '').replace(/\s+/g, ' ').trim()
  if (!cleaned) return undefined
  if (cleaned.length <= maxLen) return cleaned
  const truncated = cleaned.slice(0, maxLen)
  const lastSpace = truncated.lastIndexOf(' ')
  const breakPoint = lastSpace > 0 ? lastSpace : maxLen
  return cleaned.slice(0, breakPoint) + '\u2026'
}

/** The line a capped file read ends with, so a reader can tell it is not the whole file. */
export function truncationMarker(totalBytes?: number): string {
  return totalBytes === undefined
    ? '\n\n--- truncated ---'
    : `\n\n--- truncated (${totalBytes} bytes total) ---`
}

/** True when `text` came from a capped read and must not be saved back over the file. */
export function isTruncatedRead(text: string): boolean {
  return /\n\n--- truncated(?: \(\d+ bytes total\))? ---$/.test(text)
}
