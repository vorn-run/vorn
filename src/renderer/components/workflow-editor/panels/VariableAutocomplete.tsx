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

interface DropdownItem {
  group: string
  groupId: string
  key: string
  label: string
  description: string
  pattern: string
  disabled?: boolean
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
          disabled: group.disabled
        })
      }
    }

    for (const v of contextVars.filter((v) => v.category === 'task')) {
      items.push({
        group: 'Task',
        groupId: 'task',
        key: v.key,
        label: v.label,
        description: '',
        pattern: v.key
      })
    }

    for (const v of contextVars.filter((v) => v.category === 'trigger')) {
      items.push({
        group: 'Trigger',
        groupId: 'trigger',
        key: v.key,
        label: v.label,
        description: '',
        pattern: v.key
      })
    }

    for (const v of contextVars.filter((v) => v.category === 'context')) {
      items.push({
        group: 'Context',
        groupId: 'context',
        key: v.key,
        label: v.label,
        description: 'Resolved from the launching card or terminal',
        pattern: v.key
      })
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
                     text-white placeholder:text-gray-600 focus:outline-none focus:border-blue-500/50
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
                       ${showDropdown ? 'bg-blue-500/20 text-blue-400' : 'bg-white/[0.06] text-gray-500 hover:text-gray-300 hover:bg-white/[0.1]'}`}
            title="Insert variable (or type {{ in the editor)"
          >
            <Braces size={13} />
          </button>
        )}
      </div>

      {showDropdown && hasVariables && (
        <div
          ref={dropdownRef}
          className="absolute z-50 mt-1 w-full bg-[#2a2a2e] border border-white/[0.12]
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
                         focus:border-blue-500/40"
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

              return (
                <div key={group.id}>
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

                  {!isCollapsed &&
                    group.items.map((item) => {
                      const flatIdx = visibleItems.indexOf(item)
                      const isHighlighted = flatIdx === highlightIndex

                      return (
                        <button
                          key={item.pattern}
                          onClick={() => !item.disabled && insertPattern(item.pattern)}
                          disabled={item.disabled}
                          className={`w-full flex items-center gap-2 px-3 pl-6 py-1.5 text-left transition-colors
                                     ${item.disabled ? 'opacity-40 cursor-not-allowed' : 'cursor-pointer'}
                                     ${isHighlighted && !item.disabled ? 'bg-blue-500/15' : 'hover:bg-white/[0.04]'}`}
                        >
                          <span className="text-[12px] text-cyan-400 font-mono min-w-[50px]">
                            {item.key === item.pattern ? item.label : item.key}
                          </span>
                          {item.description && (
                            <span className="text-[11px] text-gray-600 truncate">
                              {item.description}
                            </span>
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
