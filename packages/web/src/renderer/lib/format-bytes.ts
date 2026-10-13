const UNITS = ['B', 'KB', 'MB', 'GB', 'TB']

/**
 * Human-readable byte count in binary units, matching what `du -h` prints —
 * the command someone will reach for to check these numbers by hand.
 */
export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return '0 B'

  let value = bytes
  let unit = 0
  while (value >= 1024 && unit < UNITS.length - 1) {
    value /= 1024
    unit++
  }

  // Bytes and KB never need a decimal; larger units keep one significant
  // decimal below 10 so "1.9 GB" doesn't collapse to "2 GB".
  const decimals = unit <= 1 ? 0 : value < 10 ? 1 : 0
  return `${value.toFixed(decimals)} ${UNITS[unit]}`
}
