import { useEffect, useRef } from 'react'
import { useEditor, EditorContent } from '@tiptap/react'
import StarterKit from '@tiptap/starter-kit'
import Placeholder from '@tiptap/extension-placeholder'
import TaskList from '@tiptap/extension-task-list'
import TaskItem from '@tiptap/extension-task-item'
import CodeBlockLowlight from '@tiptap/extension-code-block-lowlight'
import Link from '@tiptap/extension-link'
import { common, createLowlight } from 'lowlight'

import { FloatingToolbar } from './FloatingToolbar'
import { SlashCommands, SlashCommandMenuPortal } from './SlashCommandMenu'
import { markdownToHtml, editorJsonToMarkdown } from './markdown-utils'
import './editor-theme.css'

const lowlight = createLowlight(common)

interface RichMarkdownEditorProps {
  value: string
  onChange: (value: string) => void
  placeholder?: string
  className?: string
}

export function RichMarkdownEditor({
  value,
  onChange,
  placeholder = 'Start writing, or type / for commands...',
  className = ''
}: RichMarkdownEditorProps) {
  const skipUpdateRef = useRef(false)
  const lastValueRef = useRef(value)

  const editor = useEditor({
    extensions: [
      StarterKit.configure({
        codeBlock: false,
        horizontalRule: {
          HTMLAttributes: {}
        }
      }),
      Placeholder.configure({
        placeholder
      }),
      TaskList,
      TaskItem.configure({
        nested: true
      }),
      CodeBlockLowlight.configure({
        lowlight
      }),
      Link.configure({
        openOnClick: false,
        HTMLAttributes: {
          rel: 'noopener noreferrer nofollow'
        }
      }),
      SlashCommands
    ],
    content: markdownToHtml(value),
    onUpdate: ({ editor }) => {
      if (skipUpdateRef.current) {
        skipUpdateRef.current = false
        return
      }
      const md = editorJsonToMarkdown(editor.getJSON())
      lastValueRef.current = md
      onChange(md)
    },
    editorProps: {
      attributes: {
        class: 'focus:outline-none'
      },
      // Prevent global shortcut handlers from stealing Cmd+B etc.
      handleKeyDown: (_view, event) => {
        if ((event.metaKey || event.ctrlKey) && ['b', 'i', 'e'].includes(event.key)) {
          event.stopPropagation()
        }
        return false
      }
    }
  })

  // Sync external value changes (e.g. loading edit mode)
  useEffect(() => {
    if (!editor) return
    if (value === lastValueRef.current) return

    lastValueRef.current = value
    skipUpdateRef.current = true
    editor.commands.setContent(markdownToHtml(value))
  }, [value, editor])

  if (!editor) return null

  return (
    <div
      className={`rich-editor bg-white/[0.03] border border-white/[0.06] rounded-lg
                   focus-within:border-white/[0.15] transition-colors overflow-hidden ${className}`}
    >
      <FloatingToolbar editor={editor} />
      <EditorContent editor={editor} />
      <SlashCommandMenuPortal />
    </div>
  )
}
