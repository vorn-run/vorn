import { useEffect, useRef, useState } from 'react'
import { createPortal } from 'react-dom'
import { motion, AnimatePresence } from 'framer-motion'
import { Check, ChevronDown, Cpu, Loader2, RefreshCw } from 'lucide-react'
import type { AiAgentType } from '../../shared/types'
import {
  supportsModelSelection,
  validateModelId,
  type AgentModelCatalog,
  type AgentModelChoice
} from '@vornrun/shared/agent-models'
import { Tooltip } from './Tooltip'

/** Only used to choose a direction; the flipped menu is anchored by its edge. */
const MENU_ROW_PX = 28
const MENU_CHROME_PX = 84
const MENU_GAP_PX = 4
const VIEWPORT_MARGIN_PX = 8
const MENU_WIDTH_PX = 288

const AGENT_NAMES: Record<string, string> = {
  claude: 'claude',
  copilot: 'copilot',
  codex: 'codex',
  opencode: 'opencode'
}

interface Props {
  agentType: AiAgentType | string
  projectPath?: string
  remoteHostId?: string
  value?: string
  onChange: (model: string | undefined) => void
  /** `compact` is the launcher chip; `bordered` the intent bar's; `form` a full-width field. */
  variant?: 'compact' | 'bordered' | 'form'
  disabled?: boolean
}

function agoLabel(fetchedAt: number | undefined, now: number): string {
  if (!fetchedAt) return ''
  const minutes = Math.round((now - fetchedAt) / 60_000)
  return minutes < 1 ? 'just now' : `${minutes} min ago`
}

/** OpenCode ids are `provider/model`; the provider becomes a section header. */
function providerOf(id: string): string | undefined {
  const slash = id.indexOf('/')
  return slash > 0 ? id.slice(0, slash) : undefined
}

export function ModelPicker({
  agentType,
  projectPath,
  remoteHostId,
  value,
  onChange,
  variant = 'compact',
  disabled = false
}: Props) {
  const [open, setOpen] = useState(false)
  const [query, setQuery] = useState('')
  const [invalid, setInvalid] = useState('')
  const [catalog, setCatalog] = useState<AgentModelCatalog | null>(null)
  const [loading, setLoading] = useState(false)
  const [refreshes, setRefreshes] = useState(0)
  const [openedAt, setOpenedAt] = useState(0)
  const triggerRef = useRef<HTMLButtonElement>(null)
  const menuRef = useRef<HTMLDivElement>(null)
  const [position, setPosition] = useState<{
    top?: number
    bottom?: number
    left: number
    width: number
  }>({ top: 0, left: 0, width: MENU_WIDTH_PX })

  const supported = supportsModelSelection(agentType)
  const agentName = AGENT_NAMES[agentType] ?? agentType
  const choices = catalog?.choices ?? []
  const selected = choices.find((choice) => choice.id === value)
  const label = selected?.label ?? value

  // Asked when the menu opens, and again on refresh; the list belongs to this agent and project.
  useEffect(() => {
    if (!open || !supported) return
    let cancelled = false
    const request = {
      agentType: agentType as AiAgentType,
      projectPath: projectPath ?? '',
      remoteHostId
    }
    const list = window.api?.listAgentModels
    void (async () => {
      await Promise.resolve()
      if (cancelled) return
      if (!list) {
        setCatalog({ choices: [], status: 'unavailable', error: 'This server cannot list models.' })
        return
      }
      setLoading(true)
      try {
        const result = await list({ ...request, refresh: refreshes > 0 })
        if (!cancelled) setCatalog(result)
      } catch {
        if (!cancelled)
          setCatalog({ choices: [], status: 'unavailable', error: 'Could not list models.' })
      } finally {
        if (!cancelled) setLoading(false)
      }
    })()
    return () => {
      cancelled = true
    }
  }, [open, supported, agentType, projectPath, remoteHostId, refreshes])

  useEffect(() => {
    if (!open) return
    const handleClick = (e: MouseEvent) => {
      const target = e.target as Node
      if (triggerRef.current?.contains(target)) return
      if (menuRef.current && !menuRef.current.contains(target)) setOpen(false)
    }
    const handleKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.stopPropagation()
        setOpen(false)
        triggerRef.current?.focus()
      }
    }
    document.addEventListener('mousedown', handleClick)
    document.addEventListener('keydown', handleKey)
    return () => {
      document.removeEventListener('mousedown', handleClick)
      document.removeEventListener('keydown', handleKey)
    }
  }, [open])

  if (!supported) return null

  const handleTrigger = (e: React.MouseEvent) => {
    e.stopPropagation()
    if (disabled) return
    if (open) {
      setOpen(false)
      return
    }
    const rect = triggerRef.current?.getBoundingClientRect()
    if (rect) {
      const width = Math.min(
        Math.max(MENU_WIDTH_PX, variant === 'form' ? rect.width : 0),
        window.innerWidth - 16
      )
      const estimated = Math.min(8, choices.length + 1) * MENU_ROW_PX + MENU_CHROME_PX
      const flipUp =
        rect.bottom + MENU_GAP_PX + estimated > window.innerHeight - VIEWPORT_MARGIN_PX &&
        rect.top - MENU_GAP_PX - estimated > VIEWPORT_MARGIN_PX
      setPosition({
        top: flipUp ? undefined : rect.bottom + MENU_GAP_PX,
        bottom: flipUp ? window.innerHeight - rect.top + MENU_GAP_PX : undefined,
        left: Math.max(
          VIEWPORT_MARGIN_PX,
          Math.min(rect.left, window.innerWidth - width - VIEWPORT_MARGIN_PX)
        ),
        width
      })
    }
    setQuery('')
    setInvalid('')
    setOpenedAt(Date.now())
    setOpen(true)
  }

  const select = (model: string | undefined) => {
    setOpen(false)
    if (model !== value) onChange(model)
    triggerRef.current?.focus()
  }

  const chooseTypedId = () => {
    try {
      select(validateModelId(query))
    } catch (err) {
      setInvalid((err as Error).message)
    }
  }

  const needle = query.trim().toLowerCase()
  const shown = needle
    ? choices.filter((c) => `${c.label} ${c.id}`.toLowerCase().includes(needle))
    : choices
  const exact = shown.find((c) => c.id === needle || c.id.toLowerCase() === needle)
  const groupedByProvider = agentType === 'opencode'

  const footer = loading
    ? { text: `Asking ${agentName}…`, spinner: true }
    : catalog?.status === 'unavailable'
      ? { text: catalog.error ?? 'Could not list models — type an id', spinner: false }
      : catalog?.source === 'built-in'
        ? { text: 'Known ids — others can be typed', spinner: false }
        : catalog
          ? {
              text: `From ${agentName} · ${agoLabel(catalog.fetchedAt, openedAt)}${catalog.status === 'stale' ? ' · refreshing failed' : ''}`,
              spinner: false
            }
          : { text: '', spinner: false }

  const triggerClass =
    variant === 'form'
      ? `w-full flex items-center gap-2 px-3 py-2 text-[13px] bg-white/[0.06] border border-white/[0.1] rounded-md transition-colors ${
          disabled ? 'text-gray-600 cursor-default' : 'text-white hover:border-white/[0.2]'
        }`
      : variant === 'bordered'
        ? 'flex items-center gap-1.5 rounded-[4px] border border-white/[0.12] px-1.5 py-0.5 text-[12px] text-gray-300 transition-colors hover:border-white/25 hover:bg-white/[0.04]'
        : 'flex items-center gap-1.5 px-2 py-1 rounded-md hover:bg-white/[0.06] transition-colors text-xs text-gray-400'

  const triggerText = disabled
    ? "Follows the task's agent"
    : (label ?? (variant === 'form' ? 'Agent default' : 'Default'))
  const showText = variant !== 'bordered' || label !== undefined

  const trigger = (
    <button
      ref={triggerRef}
      type="button"
      onClick={handleTrigger}
      disabled={disabled}
      aria-haspopup="listbox"
      aria-expanded={open}
      aria-label={label ? `Model: ${label}` : 'Model: agent default'}
      className={triggerClass}
    >
      <Cpu
        size={variant === 'form' ? 14 : 13}
        className={label ? 'text-gray-300' : 'text-gray-500'}
        strokeWidth={1.75}
      />
      {showText && (
        <span
          className={`flex-1 text-left truncate max-w-[170px] ${label || disabled ? '' : 'text-gray-500'}`}
        >
          {triggerText}
        </span>
      )}
      {!disabled && showText && (
        <ChevronDown size={variant === 'form' ? 12 : 10} className="shrink-0 text-gray-500" />
      )}
    </button>
  )

  const rowClass = (current: boolean) =>
    `w-full flex items-center gap-2 px-3 py-1.5 text-[12px] text-left transition-colors ${
      current
        ? 'text-white bg-white/[0.06]'
        : 'text-gray-300 hover:text-white hover:bg-white/[0.04]'
    }`
  const checkCell = (current: boolean) =>
    current ? (
      <Check size={11} strokeWidth={3} className="shrink-0" />
    ) : (
      <span className="w-[11px] shrink-0" />
    )

  const row = (choice: AgentModelChoice) => {
    const current = choice.id === value
    return (
      <button
        key={choice.id}
        type="button"
        role="option"
        aria-selected={current}
        title={choice.description}
        onMouseDown={(e) => {
          e.preventDefault()
          e.stopPropagation()
          select(choice.id)
        }}
        className={rowClass(current)}
      >
        {checkCell(current)}
        <span className="flex-1 min-w-0 truncate">{choice.label}</span>
        {choice.label !== choice.id && (
          <span className="text-[10px] text-gray-600 font-mono shrink-0 truncate max-w-[120px]">
            {choice.id}
          </span>
        )}
      </button>
    )
  }

  const rows: React.ReactNode[] = []
  let lastProvider: string | undefined
  for (const choice of shown) {
    const provider = groupedByProvider ? providerOf(choice.id) : undefined
    if (provider && provider !== lastProvider) {
      rows.push(
        <div
          key={`p:${provider}`}
          className="px-3 pt-2 pb-0.5 text-[10px] uppercase tracking-wide text-gray-600"
        >
          {provider}
        </div>
      )
      lastProvider = provider
    }
    rows.push(row(choice))
  }

  const menu = (
    <motion.div
      ref={menuRef}
      role="listbox"
      aria-label="Model"
      initial={{ opacity: 0, scale: 0.96, y: -4 }}
      animate={{ opacity: 1, scale: 1, y: 0 }}
      exit={{ opacity: 0, scale: 0.96, y: -4 }}
      transition={{ type: 'spring', stiffness: 500, damping: 30 }}
      className="fixed z-[200] border border-white/[0.08] rounded-lg shadow-xl overflow-hidden flex flex-col"
      style={{
        background: 'var(--color-surface-overlay)',
        top: position.top,
        bottom: position.bottom,
        left: position.left,
        width: position.width,
        maxHeight: 'calc(100vh - 32px)'
      }}
    >
      <div className="overflow-y-auto py-1 max-h-60">
        {!needle && (
          <button
            type="button"
            role="option"
            aria-selected={value === undefined}
            onMouseDown={(e) => {
              e.preventDefault()
              e.stopPropagation()
              select(undefined)
            }}
            className={rowClass(value === undefined)}
          >
            {checkCell(value === undefined)}
            <span className="flex-1">Agent default</span>
          </button>
        )}
        {value && !selected && !needle && row({ id: value, label: value })}
        {rows}
        {needle && shown.length === 0 && (
          <p className="px-3 py-1.5 text-[12px] text-gray-600">No listed model matches</p>
        )}
      </div>
      <div className="border-t border-white/[0.06] px-3 py-2">
        <div className="flex items-center gap-2">
          <span className="text-[10px] uppercase tracking-wide text-gray-600 shrink-0">id</span>
          <input
            autoFocus
            value={query}
            spellCheck={false}
            aria-label="Model id"
            placeholder="type a model id"
            onChange={(e) => {
              setQuery(e.target.value)
              setInvalid('')
            }}
            onKeyDown={(e) => {
              if (e.key !== 'Enter') return
              e.preventDefault()
              e.stopPropagation()
              if (exact) select(exact.id)
              else if (needle) chooseTypedId()
            }}
            className={`flex-1 min-w-0 bg-white/[0.04] border rounded px-2 py-0.5 text-[11px] font-mono text-gray-200 placeholder:text-gray-700 focus:outline-none ${
              invalid
                ? 'border-[var(--color-danger)]'
                : 'border-white/[0.08] focus:border-white/[0.2]'
            }`}
          />
        </div>
        {invalid && <p className="mt-1 text-[11px] text-[var(--color-danger)]">{invalid}</p>}
      </div>
      <div className="flex items-center gap-2 px-3 pb-2 text-[10px] text-gray-600">
        {footer.spinner && <Loader2 size={10} className="animate-spin shrink-0" />}
        <span className="flex-1 truncate">{footer.text}</span>
        {catalog?.source === 'agent' && !loading && (
          <Tooltip label={`Ask ${agentName} again`}>
            <button
              type="button"
              aria-label="Refresh models"
              onMouseDown={(e) => {
                e.preventDefault()
                e.stopPropagation()
                setRefreshes((n) => n + 1)
              }}
              className="w-5 h-5 grid place-items-center rounded text-gray-500 hover:text-gray-300 hover:bg-white/[0.06]"
            >
              <RefreshCw size={11} />
            </button>
          </Tooltip>
        )}
      </div>
    </motion.div>
  )

  return (
    <>
      {variant === 'bordered' && !label ? (
        <Tooltip label="Model · agent default">{trigger}</Tooltip>
      ) : (
        trigger
      )}
      {createPortal(<AnimatePresence>{open && menu}</AnimatePresence>, document.body)}
    </>
  )
}
