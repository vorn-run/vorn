import { useState } from 'react'
import { createPortal } from 'react-dom'
import { motion, AnimatePresence } from 'framer-motion'
import { Check, ChevronDown, Cpu, Loader2, RefreshCw } from 'lucide-react'
import type { LaunchAgentType } from '../../shared/types'
import {
  supportsModelSelection,
  validateModelId,
  type AgentModelCatalog,
  type AgentModelChoice
} from '@vornrun/shared/agent-models'
import { Tooltip } from './Tooltip'
import { useAnchoredMenu } from '../hooks/useAnchoredMenu'
import { useAgentModelCatalog } from '../hooks/useAgentModelCatalog'

const MENU_ROW_PX = 28
const MENU_CHROME_PX = 84
const MENU_WIDTH_PX = 288

type Variant = 'compact' | 'bordered' | 'form'

interface Props {
  agentType: LaunchAgentType | string
  projectPath?: string
  remoteHostId?: string
  value?: string
  onChange: (model: string | undefined) => void
  /** `compact` is the launcher chip; `bordered` the intent bar's; `form` a full-width field. */
  variant?: Variant
  /** Ask for the list as soon as the agent and project are known, not only when opened. */
  prefetch?: boolean
}

const TRIGGER_CLASS: Record<Variant, string> = {
  form: 'w-full flex items-center gap-2 px-3 py-2 text-[13px] bg-white/[0.06] border border-white/[0.1] rounded-md transition-colors',
  bordered:
    'flex items-center gap-1.5 rounded-[4px] border border-white/[0.12] px-1.5 py-0.5 text-[12px] text-gray-300 transition-colors hover:border-white/25 hover:bg-white/[0.04]',
  compact:
    'flex items-center gap-1.5 px-2 py-1 rounded-md hover:bg-white/[0.06] transition-colors text-xs text-gray-400'
}

const DEFAULT_TEXT: Record<Variant, string> = {
  form: 'Agent default',
  bordered: '',
  compact: 'Default'
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

function footerFor(
  agent: string,
  catalog: AgentModelCatalog | null,
  loading: boolean,
  openedAt: number
): { text: string; spinner: boolean } {
  if (loading)
    return catalog ? { text: `Asking ${agent}…`, spinner: true } : { text: '', spinner: false }
  if (!catalog) return { text: '', spinner: false }
  if (catalog.status === 'unavailable') {
    return { text: catalog.error ?? 'Could not list models — type an id', spinner: false }
  }
  const failed = catalog.status === 'stale' ? ' · refreshing failed' : ''
  return {
    text: `From ${agent} · ${agoLabel(catalog.fetchedAt, openedAt)}${failed}`,
    spinner: false
  }
}

export function ModelPicker({
  agentType,
  projectPath,
  remoteHostId,
  value,
  onChange,
  variant = 'compact',
  prefetch = false
}: Props) {
  const [query, setQuery] = useState('')
  const [invalid, setInvalid] = useState('')
  const [openedAt, setOpenedAt] = useState(0)

  const followsTask = agentType === 'fromTask'
  const supported = followsTask || supportsModelSelection(agentType)

  const { open, setOpen, toggle, triggerRef, menuRef, position } = useAnchoredMenu({
    estimateHeight: () =>
      Math.min(8, (catalog?.choices.length ?? 0) + 1) * MENU_ROW_PX + MENU_CHROME_PX,
    menuWidth: (trigger) => Math.max(MENU_WIDTH_PX, variant === 'form' ? trigger : 0)
  })
  // A remote host cannot be asked, so only an opened menu learns that and says so.
  const wanted =
    supported && !followsTask && (open || (prefetch && Boolean(projectPath) && !remoteHostId))
  const { catalog, loading, refresh } = useAgentModelCatalog(
    { agentType, projectPath, remoteHostId },
    wanted
  )
  const choices = catalog?.choices ?? []
  const selected = choices.find((choice) => choice.id === value)
  const label = selected?.label ?? value

  if (!supported) return null

  const handleTrigger = (e: React.MouseEvent) => {
    if (followsTask) return
    if (!open) {
      setQuery('')
      setInvalid('')
      setOpenedAt(Date.now())
    }
    toggle(e)
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
  const exact = shown.find((c) => c.id.toLowerCase() === needle)
  const groupedByProvider = agentType === 'opencode'
  const asking = loading && !catalog
  const footer = footerFor(agentType, catalog, loading, openedAt)

  const triggerText = followsTask ? "Follows the task's agent" : (label ?? DEFAULT_TEXT[variant])
  const formTone = followsTask
    ? 'text-gray-600 cursor-default'
    : 'text-white hover:border-white/[0.2]'

  const trigger = (
    <button
      ref={triggerRef}
      type="button"
      onClick={handleTrigger}
      disabled={followsTask}
      aria-haspopup="listbox"
      aria-expanded={open}
      aria-label={label ? `Model: ${label}` : 'Model: agent default'}
      className={variant === 'form' ? `${TRIGGER_CLASS.form} ${formTone}` : TRIGGER_CLASS[variant]}
    >
      <Cpu
        size={variant === 'form' ? 14 : 13}
        className={label ? 'text-gray-300' : 'text-gray-500'}
        strokeWidth={1.75}
      />
      {triggerText && (
        <span
          className={`flex-1 text-left truncate max-w-[170px] ${label || followsTask ? '' : 'text-gray-500'}`}
        >
          {triggerText}
        </span>
      )}
      {!followsTask && triggerText && (
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
        {asking && (
          <p
            role="status"
            className="flex items-center gap-2 px-3 py-1.5 text-[12px] text-gray-500"
          >
            <Loader2 size={11} className="animate-spin shrink-0" />
            Asking {agentType} for its models…
          </p>
        )}
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
        {catalog && catalog.status !== 'unavailable' && !loading && (
          <Tooltip label={`Ask ${agentType} again`}>
            <button
              type="button"
              aria-label="Refresh models"
              onMouseDown={(e) => {
                e.preventDefault()
                e.stopPropagation()
                refresh()
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
