/**
 * Markdown <-> TipTap content conversion utilities.
 *
 * editorJsonToMarkdown: serializes ProseMirror JSON back to clean markdown.
 */

import type { JSONContent } from '@tiptap/react'

export { markdownToHtml, escapeHtml } from '@vornrun/shared/markdown'

/* ------------------------------------------------------------------ */
/*  ProseMirror JSON → Markdown                                       */
/* ------------------------------------------------------------------ */

export function editorJsonToMarkdown(doc: JSONContent): string {
  if (!doc.content) return ''
  return serializeNodes(doc.content).trim() + '\n'
}

function serializeNodes(nodes: JSONContent[]): string {
  const parts: string[] = []

  for (const node of nodes) {
    switch (node.type) {
      case 'paragraph':
        parts.push(serializeInline(node.content) + '\n')
        break

      case 'heading': {
        const level = node.attrs?.level ?? 1
        const prefix = '#'.repeat(level)
        parts.push(`${prefix} ${serializeInline(node.content)}\n`)
        break
      }

      case 'bulletList':
        if (node.content) {
          for (const item of node.content) {
            const text = serializeListItemContent(item.content)
            parts.push(`- ${text}\n`)
          }
        }
        break

      case 'orderedList':
        if (node.content) {
          node.content.forEach((item, idx) => {
            const text = serializeListItemContent(item.content)
            parts.push(`${idx + 1}. ${text}\n`)
          })
        }
        break

      case 'taskList':
        if (node.content) {
          for (const item of node.content) {
            const checked = item.attrs?.checked ? 'x' : ' '
            const text = serializeListItemContent(item.content)
            parts.push(`- [${checked}] ${text}\n`)
          }
        }
        break

      case 'codeBlock': {
        const lang = node.attrs?.language || ''
        const code = node.content?.map((n) => n.text ?? '').join('') ?? ''
        parts.push(`\`\`\`${lang}\n${code}\n\`\`\`\n`)
        break
      }

      case 'blockquote':
        if (node.content) {
          const inner = serializeNodes(node.content).trim()
          parts.push(
            inner
              .split('\n')
              .map((l) => `> ${l}`)
              .join('\n') + '\n'
          )
        }
        break

      case 'horizontalRule':
        parts.push('---\n')
        break

      case 'hardBreak':
        parts.push('\n')
        break

      default:
        // Unknown node — try inline serialization
        if (node.content) {
          parts.push(serializeInline(node.content) + '\n')
        }
    }
  }

  // Join and collapse excessive blank lines (max 2 newlines = 1 blank line)
  return parts.join('\n').replace(/\n{3,}/g, '\n\n')
}

function serializeListItemContent(content?: JSONContent[]): string {
  if (!content) return ''
  // List items contain paragraphs — serialize their inline content
  return content
    .map((child) => {
      if (child.type === 'paragraph') return serializeInline(child.content)
      return serializeNodes([child]).trim()
    })
    .join('\n')
}

function serializeInline(nodes?: JSONContent[]): string {
  if (!nodes) return ''
  return nodes.map(serializeInlineNode).join('')
}

function serializeInlineNode(node: JSONContent): string {
  if (node.type === 'hardBreak') return '\n'

  let text = node.text ?? ''
  if (!node.marks) return text

  for (const mark of node.marks) {
    switch (mark.type) {
      case 'bold':
        text = `**${text}**`
        break
      case 'italic':
        text = `*${text}*`
        break
      case 'strike':
        text = `~~${text}~~`
        break
      case 'code':
        text = `\`${text}\``
        break
      case 'link':
        text = `[${text}](${mark.attrs?.href ?? ''})`
        break
    }
  }

  return text
}
