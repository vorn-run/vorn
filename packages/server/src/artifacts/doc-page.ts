import { escapeHtml, markdownToHtml } from '@vornrun/shared/markdown'

const STYLE = `
:root{color-scheme:light dark;--bg:#ffffff;--ink:#1f2328;--muted:#5f6b76;--rule:#d8dee4;--code:#f6f8fa}
@media (prefers-color-scheme:dark){:root{--bg:#101012;--ink:#faf9f7;--muted:rgba(255,255,255,.55);--rule:rgba(255,255,255,.12);--code:#1c1c20}}
body{margin:0;background:var(--bg);color:var(--ink);font:16px/1.65 -apple-system,BlinkMacSystemFont,"Helvetica Neue",sans-serif;padding:40px 24px 80px}
main{max-width:68ch;margin:0 auto}
h1,h2,h3{line-height:1.25;margin:1.6em 0 .5em}
h1{font-size:28px;margin-top:0}h2{font-size:21px}h3{font-size:17px}
p,ul,ol,blockquote,pre{margin:0 0 1em}
a{color:inherit}
blockquote{border-left:3px solid var(--rule);padding-left:14px;color:var(--muted)}
code{font:14px ui-monospace,"SF Mono",Menlo,monospace;background:var(--code);padding:1px 4px;border-radius:3px}
pre{background:var(--code);padding:12px 14px;border-radius:4px;overflow-x:auto}
pre code{padding:0;background:none}
hr{border:0;border-top:1px solid var(--rule);margin:2em 0}
ul[data-type=taskList]{list-style:none;padding-left:0}
ul[data-type=taskList] li{display:flex;gap:8px}
ul[data-type=taskList] p{margin:0}
li p{margin:0}
`

/** A doc read as a page: its Markdown rendered in a plain reading column. */
export function renderDocPage(title: string, markdown: string): string {
  return (
    `<!doctype html><html><head><meta charset="utf-8">` +
    `<meta name="viewport" content="width=device-width,initial-scale=1">` +
    `<title>${escapeHtml(title)}</title><style>${STYLE}</style></head>` +
    `<body><main>${markdownToHtml(markdown)}</main></body></html>`
  )
}
