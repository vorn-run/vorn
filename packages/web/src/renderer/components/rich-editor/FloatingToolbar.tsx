import { useState, useCallback, useEffect, useRef } from 'react'
import { createPortal } from 'react-dom'
import type { Editor } from '@tiptap/react'
import { Bold, Italic, Strikethrough, Code, Link as LinkIcon, Unlink } from 'lucide-react'

interface FloatingToolbarProps {
  editor: Editor
}

export function FloatingToolbar({ editor }: FloatingToolbarProps) {
  const [visible, setVisible] = useState(false)
  const [position, setPosition] = useState({ top: 0, left: 0 })
  const [linkInput, setLinkInput] = useState('')
  const [showLinkInput, setShowLinkInput] = useState(false)
  const toolbarRef = useRef<HTMLDivElement>(null)

  useEffect(() => {
    const updateToolbar = () => {
      const { from, to } = editor.state.selection
      const hasSelection = from !== to
      const isCodeBlock = editor.isActive('codeBlock')

      if (!hasSelection || isCodeBlock) {
        setVisible(false)
        setShowLinkInput(false)
        return
      }

      // Get selection coordinates
      const { view } = editor
      const start = view.coordsAtPos(from)
      const end = view.coordsAtPos(to)
      const top = Math.min(start.top, end.top)
      const left = (start.left + end.left) / 2

      setPosition({ top: top - 8, left })
      setVisible(true)
    }

    editor.on('selectionUpdate', updateToolbar)

    const handleBlur = () => {
      // Delay to allow toolbar button clicks
      setTimeout(() => {
        if (!toolbarRef.current?.contains(document.activeElement)) {
          setVisible(false)
          setShowLinkInput(false)
        }
      }, 150)
    }
    editor.on('blur', handleBlur)

    return () => {
      editor.off('selectionUpdate', updateToolbar)
      editor.off('blur', handleBlur)
    }
  }, [editor])

  const toggleLink = useCallback(() => {
    if (editor.isActive('link')) {
      editor.chain().focus().unsetLink().run()
      return
    }
    const existing = editor.getAttributes('link').href ?? ''
    setLinkInput(existing)
    setShowLinkInput(true)
  }, [editor])

  const applyLink = useCallback(() => {
    if (linkInput.trim()) {
      const href = linkInput.match(/^https?:\/\//) ? linkInput : `https://${linkInput}`
      editor.chain().focus().setLink({ href }).run()
    }
    setShowLinkInput(false)
    setLinkInput('')
  }, [editor, linkInput])

  const cancelLink = useCallback(() => {
    setShowLinkInput(false)
    setLinkInput('')
    editor.commands.focus()
  }, [editor])

  if (!visible) return null

  return createPortal(
    <div
      ref={toolbarRef}
      className="fixed z-[100] animate-in fade-in"
      style={{
        top: position.top,
        left: position.left,
        transform: 'translate(-50%, -100%)'
      }}
    >
      <div
        className="flex items-center rounded-lg border border-white/[0.1] shadow-2xl overflow-hidden"
        style={{ background: 'var(--color-surface-overlay)' }}
      >
        {showLinkInput ? (
          <div className="flex items-center px-1.5 py-1 gap-1">
            <input
              type="text"
              value={linkInput}
              onChange={(e) => setLinkInput(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === 'Enter') applyLink()
                if (e.key === 'Escape') cancelLink()
              }}
              placeholder="Paste link..."
              autoFocus
              className="w-[180px] px-2 py-1 bg-white/[0.04] border border-white/[0.08] rounded text-xs
                         text-gray-200 placeholder-gray-600 focus:outline-none focus:border-white/[0.15]"
            />
            <ToolbarButton onClick={applyLink} active={false} icon={LinkIcon} title="Apply" />
          </div>
        ) : (
          <div className="flex items-center px-0.5 py-0.5 gap-0.5">
            <ToolbarButton
              onClick={() => editor.chain().focus().toggleBold().run()}
              active={editor.isActive('bold')}
              icon={Bold}
              title="Bold"
            />
            <ToolbarButton
              onClick={() => editor.chain().focus().toggleItalic().run()}
              active={editor.isActive('italic')}
              icon={Italic}
              title="Italic"
            />
            <ToolbarButton
              onClick={() => editor.chain().focus().toggleStrike().run()}
              active={editor.isActive('strike')}
              icon={Strikethrough}
              title="Strikethrough"
            />
            <ToolbarButton
              onClick={() => editor.chain().focus().toggleCode().run()}
              active={editor.isActive('code')}
              icon={Code}
              title="Code"
            />

            <div className="w-px h-4 bg-white/[0.08] mx-0.5" />

            <ToolbarButton
              onClick={toggleLink}
              active={editor.isActive('link')}
              icon={editor.isActive('link') ? Unlink : LinkIcon}
              title={editor.isActive('link') ? 'Remove link' : 'Add link'}
            />
          </div>
        )}
      </div>
    </div>,
    document.body
  )
}

function ToolbarButton({
  onClick,
  active,
  icon: Icon,
  title
}: {
  onClick: () => void
  active: boolean
  icon: React.ComponentType<{ size?: number; strokeWidth?: number }>
  title: string
}) {
  return (
    <button
      onMouseDown={(e) => {
        e.preventDefault() // Prevent editor blur
        onClick()
      }}
      title={title}
      className={`p-1.5 rounded transition-colors ${
        active
          ? 'bg-white/[0.12] text-white'
          : 'text-gray-400 hover:text-white hover:bg-white/[0.06]'
      }`}
    >
      <Icon size={14} strokeWidth={2} />
    </button>
  )
}
