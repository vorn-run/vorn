/* eslint-disable react-refresh/only-export-components, @typescript-eslint/no-explicit-any */
import { useState, useEffect, useRef } from 'react'
import { createPortal } from 'react-dom'
import { Extension } from '@tiptap/core'
import Suggestion from '@tiptap/suggestion'
import type { SuggestionKeyDownProps } from '@tiptap/suggestion'
import { filterSlashCommands, type SlashCommandItem } from './slash-commands'

/* ------------------------------------------------------------------ */
/*  Shared state between the extension and the React component        */
/* ------------------------------------------------------------------ */

type SlashMenuState = {
  items: SlashCommandItem[]
  command: ((item: SlashCommandItem) => void) | null
  clientRect: (() => DOMRect | null) | null
  visible: boolean
}

let menuState: SlashMenuState = {
  items: [],
  command: null,
  clientRect: null,
  visible: false
}
let menuListeners: Array<(s: SlashMenuState) => void> = []

function setMenuState(patch: Partial<SlashMenuState>) {
  menuState = { ...menuState, ...patch }
  menuListeners.forEach((fn) => fn(menuState))
}

let keyDownHandler: ((props: SuggestionKeyDownProps) => boolean) | null = null

/* ------------------------------------------------------------------ */
/*  TipTap Extension                                                  */
/* ------------------------------------------------------------------ */

export const SlashCommands = Extension.create({
  name: 'slashCommands',

  addOptions() {
    return {
      suggestion: {
        char: '/',
        startOfLine: false,
        items: ({ query }: { query: string }) => filterSlashCommands(query),
        command: ({
          editor,
          range,
          props
        }: {
          editor: any
          range: any
          props: SlashCommandItem
        }) => {
          editor.chain().focus().deleteRange(range).run()
          props.command(editor)
        },
        render: () => ({
          onStart: (props: any) => {
            setMenuState({
              items: props.items,
              command: (item: SlashCommandItem) => props.command({ ...props, props: item }),
              clientRect: props.clientRect,
              visible: true
            })
          },
          onUpdate: (props: any) => {
            setMenuState({
              items: props.items,
              command: (item: SlashCommandItem) => props.command({ ...props, props: item }),
              clientRect: props.clientRect
            })
          },
          onKeyDown: (props: SuggestionKeyDownProps) => {
            if (props.event.key === 'Escape') {
              setMenuState({ visible: false })
              return true
            }
            return keyDownHandler?.(props) ?? false
          },
          onExit: () => {
            setMenuState({ visible: false })
          }
        })
      }
    }
  },

  addProseMirrorPlugins() {
    return [
      Suggestion({
        editor: this.editor,
        ...this.options.suggestion
      })
    ]
  }
})

/* ------------------------------------------------------------------ */
/*  React component rendered inside RichMarkdownEditor                */
/* ------------------------------------------------------------------ */

export function SlashCommandMenuPortal() {
  const [state, setState] = useState(menuState)
  const [selectedIndex, setSelectedIndex] = useState(0)
  const containerRef = useRef<HTMLDivElement>(null)

  useEffect(() => {
    const listener = (s: SlashMenuState) => setState(s)
    menuListeners.push(listener)
    return () => {
      menuListeners = menuListeners.filter((l) => l !== listener)
    }
  }, [])

  // Reset selection when items change
  /* eslint-disable react-hooks/set-state-in-effect -- intentional reset on items change */
  useEffect(() => {
    setSelectedIndex(0)
  }, [state.items])
  /* eslint-enable react-hooks/set-state-in-effect */

  // Scroll selected item into view
  useEffect(() => {
    const el = containerRef.current?.children[selectedIndex] as HTMLElement | undefined
    el?.scrollIntoView({ block: 'nearest' })
  }, [selectedIndex])

  // Register key handler
  const { items: stateItems, command: stateCommand } = state
  useEffect(() => {
    keyDownHandler = ({ event }: SuggestionKeyDownProps) => {
      if (event.key === 'ArrowUp') {
        setSelectedIndex((prev) => (prev - 1 + stateItems.length) % stateItems.length)
        return true
      }
      if (event.key === 'ArrowDown') {
        setSelectedIndex((prev) => (prev + 1) % stateItems.length)
        return true
      }
      if (event.key === 'Enter') {
        const item = stateItems[selectedIndex]
        if (item && stateCommand) stateCommand(item)
        return true
      }
      return false
    }
    return () => {
      keyDownHandler = null
    }
  }, [stateItems, stateCommand, selectedIndex])

  if (!state.visible || state.items.length === 0) return null

  const rect = state.clientRect?.()
  if (!rect) return null

  return createPortal(
    <div
      className="fixed z-[100]"
      style={{
        left: rect.left,
        top: rect.bottom + 6
      }}
    >
      <div
        ref={containerRef}
        className="min-w-[220px] max-h-[280px] overflow-y-auto rounded-lg border border-white/[0.08] shadow-2xl py-1"
        style={{ background: 'var(--color-surface-overlay)' }}
      >
        {state.items.map((item, index) => {
          const Icon = item.icon
          return (
            <button
              key={item.title}
              onMouseDown={(e) => {
                e.preventDefault()
                if (state.command) state.command(item)
              }}
              className={`w-full flex items-center gap-3 px-3 py-2 text-left transition-colors ${
                index === selectedIndex
                  ? 'bg-white/[0.08] text-white'
                  : 'text-gray-300 hover:bg-white/[0.04] hover:text-white'
              }`}
            >
              <div
                className={`flex items-center justify-center w-7 h-7 rounded-md shrink-0 ${
                  index === selectedIndex ? 'bg-white/[0.08]' : 'bg-white/[0.04]'
                }`}
              >
                <Icon size={15} strokeWidth={1.8} />
              </div>
              <div className="min-w-0">
                <div className="text-sm font-medium truncate">{item.title}</div>
                <div className="text-[11px] text-gray-500 truncate">{item.description}</div>
              </div>
            </button>
          )
        })}
      </div>
    </div>,
    document.body
  )
}
