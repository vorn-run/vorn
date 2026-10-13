import { useState, useRef, useEffect } from 'react'

interface Props {
  value: string
  onCommit: (newName: string) => void
  onCancel: () => void
  className?: string
}

export function InlineRename({ value, onCommit, onCancel, className }: Props) {
  const [text, setText] = useState(value)
  const inputRef = useRef<HTMLInputElement>(null)

  useEffect(() => {
    const el = inputRef.current
    if (el) {
      el.focus()
      el.select()
    }
  }, [])

  const commit = (): void => {
    const trimmed = text.trim()
    if (trimmed) {
      onCommit(trimmed)
    } else {
      onCancel()
    }
  }

  const handleKeyDown = (e: React.KeyboardEvent): void => {
    e.stopPropagation()
    if (e.key === 'Enter') {
      commit()
    } else if (e.key === 'Escape') {
      onCancel()
    }
  }

  return (
    <input
      ref={inputRef}
      type="text"
      value={text}
      onChange={(e) => setText(e.target.value)}
      onBlur={commit}
      onKeyDown={handleKeyDown}
      className={`bg-white/[0.06] border border-white/[0.15] rounded px-1.5 py-0.5
                  text-gray-200 focus:outline-none focus:border-white/[0.25] ${className ?? ''}`}
    />
  )
}
