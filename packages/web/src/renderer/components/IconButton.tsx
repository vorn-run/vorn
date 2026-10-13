import type { ComponentProps, MouseEvent, ReactNode } from 'react'
import { Tooltip } from './Tooltip'

interface Props {
  /** Names the button for screen readers, and in its tooltip unless a hint says more. */
  label: string
  hint?: string
  /** The browser's own tooltip, for saying why the button is off. */
  title?: string
  disabled?: boolean
  position?: ComponentProps<typeof Tooltip>['position']
  onClick: (e: MouseEvent<HTMLButtonElement>) => void
  children: ReactNode
}

/** A neutral icon button that names what it does in a tooltip. */
export function IconButton({ label, hint, title, disabled, position, onClick, children }: Props) {
  return (
    <Tooltip label={hint ?? label} position={position}>
      <button
        type="button"
        aria-label={label}
        title={title}
        disabled={disabled}
        onClick={onClick}
        className="p-1 rounded text-ink-faint hover:text-ink hover:bg-white/[0.06] transition-colors
                   disabled:opacity-40 disabled:cursor-not-allowed disabled:hover:bg-transparent disabled:hover:text-ink-faint"
      >
        {children}
      </button>
    </Tooltip>
  )
}
