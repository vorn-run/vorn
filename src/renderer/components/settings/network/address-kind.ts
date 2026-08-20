/**
 * Which network an address reaches this machine over.
 *
 * Tailscale hands out addresses from 100.64.0.0/10, the range reserved for
 * carrier-grade NAT, and uses it for nothing else. That makes the split exact
 * rather than a guess, which matters because the two are not interchangeable:
 * a phone can only use the one whose network it is actually on, and the
 * tailnet address is the only one of them that is encrypted.
 */
export function addressKind(url: string): 'tailnet' | 'lan' {
  try {
    const host = new URL(url).hostname
    const [a, b] = host.split('.').map(Number)
    return a === 100 && b >= 64 && b <= 127 ? 'tailnet' : 'lan'
  } catch {
    return 'lan'
  }
}
