import { useEffect, useMemo, useRef, useState } from 'react'
import { Play, Terminal, GitFork, Hand, Repeat, Zap, Search } from 'lucide-react'
import type { LucideIcon } from 'lucide-react'
import { NODE_GLYPH } from '../node-visuals'
import type { AddableNodeType } from '../WorkflowCanvas'

/**
 * What picking a palette entry means: a bare step type, or a connector action
 * with the connection and action already chosen.
 */
export type PalettePick =
  | { kind: 'type'; type: AddableNodeType }
  | { kind: 'connectorAction'; connectionId: string; action: string }

export interface PaletteConnectorItem {
  connectionId: string
  action: string
  label: string
  /** Shown as the dimmed source column, e.g. the connector id. */
  source: string
}

const TYPE_ITEMS: { type: AddableNodeType; label: string; icon: LucideIcon }[] = [
  { type: 'agent', label: 'Add an agent', icon: Play },
  { type: 'script', label: 'Add a script', icon: Terminal },
  { type: 'condition', label: 'Add a condition', icon: GitFork },
  { type: 'approval', label: 'Add an approval gate', icon: Hand },
  { type: 'loop', label: 'Repeat steps until…', icon: Repeat },
  { type: 'connectorAction', label: 'Call a connector action', icon: Zap }
]

/**
 * The node search panel: opened by dropping a connection on empty canvas or
 * pressing Tab. Lists the step types and every installed connector action,
 * filtered as the user types; Enter picks the highlighted row.
 */
export function NodePalette({
  position,
  allowLoop,
  connectorItems,
  onPick,
  onClose
}: {
  /** Where to place the panel, in the canvas container's coordinates. */
  position: { x: number; y: number }
  /** Loops lift their body out of the trunk; inside a branch that is untested. */
  allowLoop: boolean
  connectorItems: PaletteConnectorItem[]
  onPick: (pick: PalettePick) => void
  onClose: () => void
}) {
  const [query, setQuery] = useState('')
  const [highlight, setHighlight] = useState(0)
  const rootRef = useRef<HTMLDivElement>(null)
  const inputRef = useRef<HTMLInputElement>(null)

  useEffect(() => {
    inputRef.current?.focus()
  }, [])

  useEffect(() => {
    function handlePointer(e: MouseEvent) {
      if (rootRef.current && !rootRef.current.contains(e.target as Node)) onClose()
    }
    document.addEventListener('mousedown', handlePointer)
    return () => document.removeEventListener('mousedown', handlePointer)
  }, [onClose])

  const items = useMemo(() => {
    const q = query.trim().toLowerCase()
    const types = TYPE_ITEMS.filter((t) => allowLoop || t.type !== 'loop')
      .filter((t) => !q || t.label.toLowerCase().includes(q))
      .map((t) => ({
        key: `type:${t.type}`,
        label: t.label,
        source: '',
        icon: t.icon,
        pick: { kind: 'type', type: t.type } as PalettePick
      }))
    const actions = connectorItems
      .filter((c) => !q || c.label.toLowerCase().includes(q) || c.source.toLowerCase().includes(q))
      .map((c) => ({
        key: `action:${c.connectionId}:${c.action}`,
        label: c.label,
        source: c.source,
        icon: Zap,
        pick: {
          kind: 'connectorAction',
          connectionId: c.connectionId,
          action: c.action
        } as PalettePick
      }))
    return [...types, ...actions]
  }, [query, allowLoop, connectorItems])

  const clampedHighlight = Math.min(highlight, Math.max(0, items.length - 1))

  return (
    <div
      ref={rootRef}
      data-node-palette
      className="absolute z-50 w-[248px] bg-surface-overlay border border-white/[0.12]
                 rounded-lg shadow-xl shadow-black/40 p-1.5
                 animate-in fade-in-0 zoom-in-95 duration-100"
      style={{ left: position.x, top: position.y }}
      onKeyDown={(e) => {
        if (e.key === 'Escape') {
          e.stopPropagation()
          onClose()
        } else if (e.key === 'ArrowDown') {
          e.preventDefault()
          setHighlight((h) => Math.min(h + 1, items.length - 1))
        } else if (e.key === 'ArrowUp') {
          e.preventDefault()
          setHighlight((h) => Math.max(h - 1, 0))
        } else if (e.key === 'Enter' && items[clampedHighlight]) {
          e.preventDefault()
          onPick(items[clampedHighlight].pick)
        }
      }}
    >
      <div className="flex items-center gap-1.5 border border-white/[0.08] rounded-md px-2 py-1.5 mb-1.5">
        <Search size={12} className="shrink-0 text-gray-500" />
        <input
          ref={inputRef}
          value={query}
          onChange={(e) => {
            setQuery(e.target.value)
            setHighlight(0)
          }}
          placeholder="Search steps and actions"
          className="w-full bg-transparent text-[12px] text-white placeholder:text-gray-600 outline-none"
        />
      </div>

      <div className="max-h-[264px] overflow-y-auto">
        {items.length === 0 && (
          <div className="px-2.5 py-3 text-[11px] text-gray-500">Nothing matches</div>
        )}
        {items.map((item, i) => {
          const Icon = item.icon
          return (
            <button
              key={item.key}
              onClick={() => onPick(item.pick)}
              onMouseEnter={() => setHighlight(i)}
              className={`w-full flex items-center gap-2.5 px-2.5 py-2 rounded-md text-[13px]
                          text-left transition-colors
                          ${i === clampedHighlight ? 'bg-white/[0.06] text-white' : 'text-gray-300'}`}
            >
              <Icon size={14} className={`${NODE_GLYPH} shrink-0`} />
              <span className="truncate flex-1">{item.label}</span>
              {item.source && (
                <span className="shrink-0 text-[10px] font-mono text-gray-500 truncate max-w-[80px]">
                  {item.source}
                </span>
              )}
            </button>
          )
        })}
      </div>
    </div>
  )
}
