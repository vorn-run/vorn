import {
  useState,
  useRef,
  useEffect,
  useCallback,
  useMemo,
  type KeyboardEvent,
  type ChangeEvent
} from 'react'
import { Braces, ChevronDown, ChevronRight } from 'lucide-react'
import type { StepVariableGroup, TemplateVariable } from '../../../lib/template-vars'
import { previewStepTokens } from '../../../lib/template-vars'
import { NODE_TYPE_ICON } from '../node-visuals'
import { WORKFLOW_STATUS_DOT } from '../../../lib/workflow-status'
import { ConnectorIcon } from '../../ConnectorIcon'
import { useConnectorIdFor, useConnectionIconFor } from '../../../lib/use-connections'
import type { WorkflowNode } from '../../../../shared/types'

interface Props {
  value: string
  onChange: (value: string) => void
  placeholder?: string
  rows?: number
  stepGroups: StepVariableGroup[]
  contextVars: TemplateVariable[]
  className?: string
  mono?: boolean
}

/**
 * How each variable category is presented in the picker, in display order.
 *
 * A Record over the category union rather than a loop per category: the picker
 * previously listed three categories by hand, so anything else a form passed
 * in — connector item fields, and later run inputs — was silently dropped from
 * the dropdown while still resolving at run time. Adding a category to
 * TemplateVariable now fails to compile until it is given a group here.
 */
const VAR_CATEGORY_GROUPS: Record<
  TemplateVariable['category'],
  { group: string; description: string }
> = {
  task: { group: 'Task', description: '' },
  trigger: { group: 'Trigger', description: '' },
  context: { group: 'Context', description: 'Resolved from the launching card or terminal' },
  inputs: { group: 'Run Inputs', description: 'Entered when the run is started' },
  connectorItem: {
    group: 'Connector Item',
    description: 'The upstream item that triggered this run'
  }
}

interface DropdownItem {
  group: string
  groupId: string
  key: string
  label: string
  description: string
  pattern: string
  disabled?: boolean
  value?: string
}

/** The step's own glyph — its connector's brand mark when it has one. */
function StepGroupIcon({ group }: { group: StepVariableGroup }) {
  const connectorId = useConnectorIdFor(group.connectionId ?? null)
  const icon = useConnectionIconFor(group.connectionId ?? null)
  if (connectorId) {
    return (
      <ConnectorIcon
        connectorId={connectorId}
        icon={icon}
        size={12}
        className="text-ink-secondary shrink-0"
      />
    )
  }
  const Icon = NODE_TYPE_ICON[group.nodeType as WorkflowNode['type']]
  return Icon ? <Icon size={12} className="text-ink-secondary shrink-0" strokeWidth={2} /> : null
}

function ranAtLabel(iso: string | undefined): string | undefined {
  if (!iso) return undefined
  const date = new Date(iso)
  return isNaN(date.getTime())
    ? undefined
    : date.toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' })
}

export function VariableAutocomplete({
  value,
  onChange,
  placeholder,
  rows = 4,
  stepGroups,
  contextVars,
  className,
  mono
}: Props) {
  const textareaRef = useRef<HTMLTextAreaElement>(null)
  const dropdownRef = useRef<HTMLDivElement>(null)
  const [showDropdown, setShowDropdown] = useState(false)
  const [filter, setFilter] = useState('')
  const [triggerPos, setTriggerPos] = useState<number | null>(null)
  const [collapsedGroups, setCollapsedGroups] = useState<Set<string>>(new Set())
  const [highlightIndex, setHighlightIndex] = useState(0)

  const allItems = useMemo<DropdownItem[]>(() => {
    const items: DropdownItem[] = []

    for (const group of stepGroups) {
      for (const k of group.keys) {
        items.push({
          group: group.label,
          groupId: group.slug,
          key: k.key,
          label: k.label,
          description: k.description,
          pattern: `{{steps.${group.slug}.${k.key}}}`,
          disabled: group.disabled,
          value: k.value
        })
      }
    }

    for (const [category, { group, description }] of Object.entries(VAR_CATEGORY_GROUPS) as [
      TemplateVariable['category'],
      { group: string; description: string }
    ][]) {
      for (const v of contextVars.filter((v) => v.category === category)) {
        items.push({
          group,
          groupId: category,
          key: v.key,
          label: v.label,
          description,
          pattern: v.key
        })
      }
    }

    return items
  }, [stepGroups, contextVars])

  const filteredItems = useMemo(() => {
    if (!filter) return allItems
    const lf = filter.toLowerCase()
    return allItems.filter(
      (item) =>
        item.label.toLowerCase().includes(lf) ||
        item.group.toLowerCase().includes(lf) ||
        item.key.toLowerCase().includes(lf)
    )
  }, [allItems, filter])

  const visibleItems = useMemo(() => {
    return filteredItems.filter((item) => !collapsedGroups.has(item.groupId))
  }, [filteredItems, collapsedGroups])

  const groupedItems = useMemo(() => {
    const groups: { id: string; name: string; items: DropdownItem[]; disabled?: boolean }[] = []
    let currentGroupId = ''

    for (const item of filteredItems) {
      if (item.groupId !== currentGroupId) {
        currentGroupId = item.groupId
        groups.push({ id: item.groupId, name: item.group, items: [], disabled: item.disabled })
      }
      groups[groups.length - 1].items.push(item)
    }

    return groups
  }, [filteredItems])

  useEffect(() => {
    setHighlightIndex(0) // eslint-disable-line react-hooks/set-state-in-effect
  }, [filter])

  const insertPattern = useCallback(
    (pattern: string) => {
      const el = textareaRef.current
      if (!el) {
        onChange(value + pattern)
        setShowDropdown(false)
        return
      }

      const start = triggerPos != null ? triggerPos : el.selectionStart
      const end = el.selectionEnd
      const newValue = value.slice(0, start) + pattern + value.slice(end)
      onChange(newValue)
      setShowDropdown(false)
      setFilter('')
      setTriggerPos(null)

      requestAnimationFrame(() => {
        const newCursor = start + pattern.length
        el.selectionStart = el.selectionEnd = newCursor
        el.focus()
      })
    },
    [value, onChange, triggerPos]
  )

  const handleChange = useCallback(
    (e: ChangeEvent<HTMLTextAreaElement>) => {
      const newValue = e.target.value
      onChange(newValue)

      const cursor = e.target.selectionStart
      const textBefore = newValue.slice(0, cursor)

      if (textBefore.endsWith('{{')) {
        setTriggerPos(cursor - 2)
        setShowDropdown(true)
        setFilter('')
        return
      }

      if (showDropdown && triggerPos != null) {
        const textAfterTrigger = newValue.slice(triggerPos + 2, cursor)
        if (textAfterTrigger.includes('}}')) {
          setShowDropdown(false)
          setFilter('')
          setTriggerPos(null)
        } else {
          setFilter(textAfterTrigger)
        }
      }
    },
    [onChange, showDropdown, triggerPos]
  )

  const handleKeyDown = useCallback(
    (e: KeyboardEvent<HTMLTextAreaElement>) => {
      if (!showDropdown) return

      if (e.key === 'Escape') {
        e.preventDefault()
        setShowDropdown(false)
        setFilter('')
        setTriggerPos(null)
        return
      }

      if (e.key === 'ArrowDown') {
        e.preventDefault()
        setHighlightIndex((prev) => Math.min(prev + 1, visibleItems.length - 1))
        return
      }

      if (e.key === 'ArrowUp') {
        e.preventDefault()
        setHighlightIndex((prev) => Math.max(prev - 1, 0))
        return
      }

      if (e.key === 'Enter' || e.key === 'Tab') {
        const item = visibleItems[highlightIndex]
        if (item && !item.disabled) {
          e.preventDefault()
          insertPattern(item.pattern)
        }
      }
    },
    [showDropdown, visibleItems, highlightIndex, insertPattern]
  )

  useEffect(() => {
    if (!showDropdown) return
    function handleClick(e: MouseEvent) {
      if (
        dropdownRef.current &&
        !dropdownRef.current.contains(e.target as Node) &&
        textareaRef.current &&
        !textareaRef.current.contains(e.target as Node)
      ) {
        setShowDropdown(false)
        setFilter('')
        setTriggerPos(null)
      }
    }
    document.addEventListener('mousedown', handleClick)
    return () => document.removeEventListener('mousedown', handleClick)
  }, [showDropdown])

  const toggleGroup = (name: string) => {
    setCollapsedGroups((prev) => {
      const next = new Set(prev)
      if (next.has(name)) next.delete(name)
      else next.add(name)
      return next
    })
  }

  const hasVariables = allItems.length > 0

  const groupsBySlug = useMemo(() => new Map(stepGroups.map((g) => [g.slug, g])), [stepGroups])

  const preview = useMemo(
    () => (value.includes('{{') ? previewStepTokens(value, stepGroups) : undefined),
    [value, stepGroups]
  )

  return (
    <div className="relative">
      <div className="relative">
        <textarea
          ref={textareaRef}
          value={value}
          onChange={handleChange}
          onKeyDown={handleKeyDown}
          placeholder={placeholder}
          rows={rows}
          spellCheck={false}
          className={`w-full px-3 py-2 text-[13px] bg-white/[0.06] border border-white/[0.1] rounded-md
                     text-white placeholder:text-gray-600 focus:outline-none focus:border-white/[0.2]
                     resize-none ${mono ? 'font-mono text-[12px]' : ''} ${className || ''}`}
        />

        {hasVariables && (
          <button
            onClick={(e) => {
              e.preventDefault()
              e.stopPropagation()
              if (showDropdown) {
                setShowDropdown(false)
              } else {
                const el = textareaRef.current
                setTriggerPos(el ? el.selectionStart : value.length)
                setShowDropdown(true)
                setFilter('')
              }
            }}
            className={`absolute top-1.5 right-1.5 p-1 rounded transition-colors
                       ${showDropdown ? 'bg-white/[0.10] text-ink' : 'bg-white/[0.06] text-gray-500 hover:text-gray-300 hover:bg-white/[0.1]'}`}
            title="Insert variable (or type {{ in the editor)"
          >
            <Braces size={13} />
          </button>
        )}
      </div>

      {preview?.broken && (
        <p className="text-[10.5px] font-mono text-danger/90 mt-1 truncate">
          {preview.broken.token} not found
          {preview.broken.suggestion ? ` — did you mean ${preview.broken.suggestion}?` : ''}
        </p>
      )}
      {preview?.resolved && (
        <p className="text-[10.5px] font-mono text-ink-faint mt-1 truncate">
          → {preview.resolved.replace(/\s+/g, ' ').slice(0, 160)}
        </p>
      )}

      {showDropdown && hasVariables && (
        <div
          ref={dropdownRef}
          className="absolute z-50 mt-1 w-full bg-surface-overlay border border-white/[0.12]
                     rounded-lg shadow-xl shadow-black/40 overflow-hidden
                     animate-in fade-in-0 zoom-in-95 duration-100"
          style={{ maxHeight: 280 }}
        >
          <div className="px-2.5 py-2 border-b border-white/[0.08]">
            <input
              type="text"
              value={filter}
              onChange={(e) => setFilter(e.target.value)}
              placeholder="Filter variables..."
              className="w-full px-2 py-1 text-[12px] bg-white/[0.06] border border-white/[0.08]
                         rounded text-gray-300 placeholder:text-gray-600 focus:outline-none
                         focus:border-white/[0.2]"
              autoFocus={false}
            />
          </div>

          <div className="overflow-y-auto" style={{ maxHeight: 230 }}>
            {groupedItems.length === 0 && (
              <div className="px-3 py-3 text-[12px] text-gray-600 text-center">
                No matching variables
              </div>
            )}

            {groupedItems.map((group) => {
              const isCollapsed = collapsedGroups.has(group.id)
              const Chevron = isCollapsed ? ChevronRight : ChevronDown

              const stepGroup = groupsBySlug.get(group.id)
              const ranAt = ranAtLabel(stepGroup?.runCompletedAt)

              return (
                <div key={group.id}>
                  {stepGroup ? (
                    <button
                      onClick={() => toggleGroup(group.id)}
                      className="w-full flex items-center gap-2 px-2.5 py-1.5 transition-colors
                                 text-gray-400 hover:text-gray-300 hover:bg-white/[0.03]"
                    >
                      <Chevron size={11} className="shrink-0" />
                      <StepGroupIcon group={stepGroup} />
                      <span className="text-[11.5px] font-semibold text-ink-secondary truncate">
                        {group.name}
                      </span>
                      {stepGroup.runStatus && WORKFLOW_STATUS_DOT[stepGroup.runStatus] && (
                        <span
                          className={`shrink-0 w-1.5 h-1.5 rounded-full ${WORKFLOW_STATUS_DOT[stepGroup.runStatus]}`}
                        />
                      )}
                      {ranAt && (
                        <span className="ml-auto text-[9.5px] font-mono text-gray-600 shrink-0">
                          {ranAt}
                        </span>
                      )}
                    </button>
                  ) : (
                    <button
                      onClick={() => toggleGroup(group.id)}
                      className={`w-full flex items-center gap-1.5 px-2.5 py-1.5 text-[10px] font-semibold
                                 uppercase tracking-wider transition-colors
                                 ${group.disabled ? 'text-gray-600' : 'text-gray-500 hover:text-gray-400 hover:bg-white/[0.03]'}`}
                    >
                      <Chevron size={11} />
                      {group.name}
                      {group.disabled && (
                        <span className="text-[9px] font-normal normal-case tracking-normal text-gray-600 ml-1">
                          (no output)
                        </span>
                      )}
                    </button>
                  )}

                  {!isCollapsed &&
                    group.items.map((item) => {
                      const flatIdx = visibleItems.indexOf(item)
                      const isHighlighted = flatIdx === highlightIndex

                      return (
                        <button
                          key={`${item.groupId}:${item.key}`}
                          onClick={() => !item.disabled && insertPattern(item.pattern)}
                          disabled={item.disabled}
                          className={`w-full flex items-center gap-2 px-3 pl-6 py-1.5 text-left transition-colors
                                     ${item.disabled ? 'opacity-40 cursor-not-allowed' : 'cursor-pointer'}
                                     ${isHighlighted && !item.disabled ? 'bg-white/[0.08]' : 'hover:bg-white/[0.04]'}`}
                        >
                          <span className="text-[12px] text-ink-secondary font-mono min-w-[50px]">
                            {item.key === item.pattern ? item.label : item.key}
                          </span>
                          {item.value ? (
                            <span className="ml-auto text-[10.5px] font-mono text-gray-500 truncate max-w-[45%]">
                              {item.value}
                            </span>
                          ) : (
                            item.description && (
                              <span className="text-[11px] text-gray-600 truncate">
                                {item.description}
                              </span>
                            )
                          )}
                        </button>
                      )
                    })}
                </div>
              )
            })}
          </div>
        </div>
      )}
    </div>
  )
}
