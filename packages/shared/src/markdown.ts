/**
 * Markdown to semantic HTML, for the rich editor to load and for a doc artifact to be read as a page.
 *
 * Text is escaped before inline marks are applied, so Markdown can never smuggle raw HTML through.
 */

export function markdownToHtml(md: string): string {
  if (!md.trim()) return ''

  const lines = md.split('\n')
  const htmlParts: string[] = []
  let i = 0

  while (i < lines.length) {
    const line = lines[i]

    // Code blocks
    if (line.startsWith('```')) {
      const lang = line.slice(3).trim()
      const codeLines: string[] = []
      i++
      while (i < lines.length && !lines[i].startsWith('```')) {
        codeLines.push(escapeHtml(lines[i]))
        i++
      }
      i++ // skip closing ```
      const langAttr = lang ? ` class="language-${escapeHtml(lang)}"` : ''
      htmlParts.push(`<pre><code${langAttr}>${codeLines.join('\n')}</code></pre>`)
      continue
    }

    // Headings
    const headingMatch = line.match(/^(#{1,3}) (.+)$/)
    if (headingMatch) {
      const level = headingMatch[1].length
      htmlParts.push(`<h${level}>${inlineMarkdown(headingMatch[2])}</h${level}>`)
      i++
      continue
    }

    // Horizontal rule
    if (/^---+$/.test(line.trim())) {
      htmlParts.push('<hr>')
      i++
      continue
    }

    // Blockquote
    if (line.startsWith('> ')) {
      const quoteLines: string[] = []
      while (i < lines.length && lines[i].startsWith('> ')) {
        quoteLines.push(lines[i].slice(2))
        i++
      }
      htmlParts.push(
        `<blockquote><p>${quoteLines.map(inlineMarkdown).join('<br>')}</p></blockquote>`
      )
      continue
    }

    // Task list items
    const taskMatch = line.match(/^- \[([ x])\] (.+)$/)
    if (taskMatch) {
      const items: string[] = []
      while (i < lines.length) {
        const tm = lines[i].match(/^- \[([ x])\] (.+)$/)
        if (!tm) break
        const checked = tm[1] === 'x' ? 'true' : 'false'
        items.push(
          `<li data-type="taskItem" data-checked="${checked}"><label><input type="checkbox"${tm[1] === 'x' ? ' checked' : ''}><span></span></label><div><p>${inlineMarkdown(tm[2])}</p></div></li>`
        )
        i++
      }
      htmlParts.push(`<ul data-type="taskList">${items.join('')}</ul>`)
      continue
    }

    // Unordered list
    if (/^- .+/.test(line)) {
      const items: string[] = []
      while (i < lines.length && /^- .+/.test(lines[i])) {
        items.push(`<li><p>${inlineMarkdown(lines[i].slice(2))}</p></li>`)
        i++
      }
      htmlParts.push(`<ul>${items.join('')}</ul>`)
      continue
    }

    // Ordered list
    if (/^\d+\. .+/.test(line)) {
      const items: string[] = []
      while (i < lines.length && /^\d+\. .+/.test(lines[i])) {
        const text = lines[i].replace(/^\d+\. /, '')
        items.push(`<li><p>${inlineMarkdown(text)}</p></li>`)
        i++
      }
      htmlParts.push(`<ol>${items.join('')}</ol>`)
      continue
    }

    // Empty line
    if (line.trim() === '') {
      i++
      continue
    }

    // Paragraph
    htmlParts.push(`<p>${inlineMarkdown(line)}</p>`)
    i++
  }

  return htmlParts.join('')
}

/** Escape HTML special characters to prevent XSS injection. */
export function escapeHtml(str: string): string {
  return str
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&#39;')
}

function inlineMarkdown(text: string): string {
  return escapeHtml(text)
    .replace(/\*\*(.+?)\*\*/g, '<strong>$1</strong>')
    .replace(/\*(.+?)\*/g, '<em>$1</em>')
    .replace(/~~(.+?)~~/g, '<s>$1</s>')
    .replace(/`([^`]+)`/g, '<code>$1</code>')
    .replace(/\[([^\]]+)\]\(([^)]+)\)/g, (_, label: string, href: string) =>
      /^(https?:|mailto:|#)/i.test(href) ? `<a href="${href}">${label}</a>` : label
    )
}
