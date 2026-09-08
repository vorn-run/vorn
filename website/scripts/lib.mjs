// What the asset scripts share: where assets live, and how a raster becomes a webp.
import { execFileSync } from 'node:child_process'

export const ASSETS = new URL('../assets/', import.meta.url).pathname
export const QUALITY = '82'

export function toWebp(src, width, out, quality = QUALITY) {
  execFileSync('cwebp', ['-quiet', '-q', quality, '-resize', String(width), '0', src, '-o', out])
  return out
}
