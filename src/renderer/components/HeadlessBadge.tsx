import { useRef, useState } from 'react'
import { Zap } from 'lucide-react'
import { useFilteredHeadless } from '../hooks/useFilteredHeadless'
import { useOutsideClick } from '../hooks/useOutsideClick'
import { HeadlessPill } from './HeadlessPill'
import { Tooltip } from './Tooltip'

interface Props {
  align?: 'left' | 'right'
}

export function HeadlessBadge({ align = 'left' }: Props) {
  const headlessSessions = useFilteredHeadless()
  const [open, setOpen] = useState(false)
  const ref = useRef<HTMLDivElement>(null)
  useOutsideClick(ref, open, () => setOpen(false))

  if (headlessSessions.length === 0) return null

  let runningCount = 0
  let errorCount = 0
  for (const s of headlessSessions) {
    if (s.status === 'running') runningCount++
    else if (s.status === 'exited' && s.exitCode !== 0) errorCount++
  }
  const total = headlessSessions.length

  const breakdownParts = [`${total} headless`]
  if (runningCount > 0) breakdownParts.push(`${runningCount} running`)
  else if (errorCount > 0) breakdownParts.push(`${errorCount} errored`)
  const tooltip = breakdownParts.join(' · ')

  return (
    <div ref={ref} className="relative flex items-center titlebar-no-drag">
      <Tooltip label={tooltip} position="bottom">
        <button
          type="button"
          onClick={() => setOpen((v) => !v)}
          className="inline-flex items-center gap-1.5 h-[26px] px-2
                     rounded-md border border-white/[0.06] bg-surface-overlay
                     text-[11px] font-medium text-gray-300
                     hover:text-white hover:border-white/[0.12] transition-colors
                     relative"
          aria-haspopup="dialog"
          aria-expanded={open}
          aria-label={tooltip}
        >
          <Zap size={12} strokeWidth={1.5} />
          <span className="font-mono leading-none">{total}</span>
          {runningCount > 0 && (
            <span
              aria-hidden="true"
              className="absolute -top-0.5 -right-0.5 w-1.5 h-1.5 rounded-full bg-green-500 animate-pulse"
            />
          )}
          {runningCount === 0 && errorCount > 0 && (
            <span
              aria-hidden="true"
              className="absolute -top-0.5 -right-0.5 w-1.5 h-1.5 rounded-full bg-red-500"
            />
          )}
        </button>
      </Tooltip>

      {open && (
        <div
          role="dialog"
          aria-label="Headless sessions"
          className={`absolute top-full ${align === 'left' ? 'left-0' : 'right-0'} mt-1.5 z-50 p-1.5
                     flex flex-col gap-1.5 max-h-[60vh] overflow-y-auto min-w-[280px]
                     bg-surface-overlay border border-white/[0.08] rounded-md shadow-lg`}
        >
          {headlessSessions.map((session) => (
            <HeadlessPill key={session.id} session={session} />
          ))}
        </div>
      )}
    </div>
  )
}
