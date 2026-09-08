// Stamp the latest stable release into the page, so the badge and the
// direct download links are right before the browser's own fetch runs
// (and when it cannot: rate limit, offline, a blocked API).
// Usage: node scripts/stamp.mjs   then commit index.html
import { readFileSync, writeFileSync } from 'node:fs'

const REPO = 'vorn-run/vorn'
const PAGE = new URL('../index.html', import.meta.url).pathname
const MATCH = {
  mac: (n) => n.endsWith('.dmg') && !n.endsWith('.blockmap'),
  win: (n) => /-Setup-.*\.exe$/.test(n) && !n.endsWith('.blockmap'),
  linux: (n) => n.endsWith('.AppImage')
}

const res = await fetch(`https://api.github.com/repos/${REPO}/releases/latest`, {
  headers: { Accept: 'application/vnd.github+json', 'User-Agent': 'vorn-site-stamp' }
})
if (!res.ok) throw new Error(`releases/latest ${res.status}`)
const release = await res.json()
let html = readFileSync(PAGE, 'utf8')

html = html.replace(
  /(id="version-badge">\s*<span class="dot"><\/span>\s*)v[\w.-]+/,
  `$1${release.tag_name}`
)
for (const [os, match] of Object.entries(MATCH)) {
  const asset = (release.assets ?? []).find((a) => match(a.name))
  if (!asset) continue
  const re = new RegExp(`(data-os="${os}"[^>]*href=")[^"]*(")`)
  if (!re.test(html)) {
    // href comes before data-os in the markup: try the other order.
    const re2 = new RegExp(`(href=")[^"]*("[^>]*data-os="${os}")`)
    html = html.replace(re2, `$1${asset.browser_download_url}$2`)
  } else html = html.replace(re, `$1${asset.browser_download_url}$2`)
}
writeFileSync(PAGE, html)
console.log(`stamped ${release.tag_name}`)
