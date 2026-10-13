import { getShortcut } from '../lib/keyboard-shortcuts'

interface Props {
  shortcutId: string
  className?: string
}

export function KbdHint({ shortcutId, className }: Props) {
  const shortcut = getShortcut(shortcutId)
  if (!shortcut) return null

  return (
    <kbd
      className={`inline-flex items-center justify-center px-1.5 py-0.5 text-[10px] font-mono
                  text-gray-500 bg-white/[0.04] border border-white/[0.06] rounded
                  leading-none ${className ?? ''}`}
    >
      {shortcut.display}
    </kbd>
  )
}
