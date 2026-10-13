import type { Editor } from '@tiptap/react'
import {
  Heading1,
  Heading2,
  Heading3,
  List,
  ListOrdered,
  CheckSquare,
  Code,
  Quote,
  Minus,
  type LucideIcon
} from 'lucide-react'

export interface SlashCommandItem {
  title: string
  description: string
  icon: LucideIcon
  command: (editor: Editor) => void
}

export const slashCommandItems: SlashCommandItem[] = [
  {
    title: 'Heading 1',
    description: 'Large section heading',
    icon: Heading1,
    command: (editor) => editor.chain().focus().toggleHeading({ level: 1 }).run()
  },
  {
    title: 'Heading 2',
    description: 'Medium section heading',
    icon: Heading2,
    command: (editor) => editor.chain().focus().toggleHeading({ level: 2 }).run()
  },
  {
    title: 'Heading 3',
    description: 'Small section heading',
    icon: Heading3,
    command: (editor) => editor.chain().focus().toggleHeading({ level: 3 }).run()
  },
  {
    title: 'Bullet List',
    description: 'Unordered list of items',
    icon: List,
    command: (editor) => editor.chain().focus().toggleBulletList().run()
  },
  {
    title: 'Numbered List',
    description: 'Ordered list of items',
    icon: ListOrdered,
    command: (editor) => editor.chain().focus().toggleOrderedList().run()
  },
  {
    title: 'Task List',
    description: 'Checklist with checkboxes',
    icon: CheckSquare,
    command: (editor) => editor.chain().focus().toggleTaskList().run()
  },
  {
    title: 'Code Block',
    description: 'Fenced code snippet',
    icon: Code,
    command: (editor) => editor.chain().focus().toggleCodeBlock().run()
  },
  {
    title: 'Blockquote',
    description: 'Quoted text block',
    icon: Quote,
    command: (editor) => editor.chain().focus().toggleBlockquote().run()
  },
  {
    title: 'Divider',
    description: 'Horizontal separator line',
    icon: Minus,
    command: (editor) => editor.chain().focus().setHorizontalRule().run()
  }
]

export function filterSlashCommands(query: string): SlashCommandItem[] {
  if (!query) return slashCommandItems
  const lower = query.toLowerCase()
  return slashCommandItems.filter(
    (item) =>
      item.title.toLowerCase().includes(lower) || item.description.toLowerCase().includes(lower)
  )
}
