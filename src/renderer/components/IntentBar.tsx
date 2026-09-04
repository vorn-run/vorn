import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  useSyncExternalStore,
  type KeyboardEvent
} from 'react'
import { readIntentDraft, writeIntentDraft } from '../lib/intent-drafts'
import { createPortal } from 'react-dom'
import {
  CornerDownLeft,
  CornerDownRight,
  File,
  Folder,
  GitBranch,
  History,
  Play,
  SlidersHorizontal,
  Terminal
} from 'lucide-react'
import { useAppStore } from '../stores'
import { focusTerminal, pasteToTerminal, scrollToBottom } from '../lib/terminal-registry'
import { getShellInputState, isAtPrompt, onCommandBlocksChange } from '../lib/command-blocks'
import { useIsMobile } from '../hooks/useIsMobile'
import {
  ghostSuggestion,
  recordCommand,
  searchHistory,
  type CommandHistoryEntry,
  type HistoryKind
} from '../lib/command-history'
import { defaultSources, getCompletions, outlineNames, type Completion } from '../lib/completions'
import { registerIntentBarInput } from '../lib/intent-bar-focus'
import { isShellSession } from '../../shared/types'
import { resolveIntentMode, SHELL_BUILTINS, type IntentMode } from '../lib/intent-mode'
import { getPreferredAgent, setPreferredAgent } from '../lib/launch-prefs'
import { getDisplayPathBasename, launchAgentFromShell } from '../lib/session-utils'
import { AgentPicker } from './AgentPicker'
import { useAgentInstallStatus } from '../hooks/useAgentInstallStatus'
import type { AiAgentType, LaunchAgentType } from '../../shared/types'

interface Props {
  terminalId: string
  /** Grid-card variant: single row, no context strip. */
  compact?: boolean
  /**
   * Left inset in pixels, so the caret lands in the same column as the
   * terminal's own text. Derived by the caller from the spine geometry plus
   * whatever padding its container adds.
   */
  indentPx?: number
}

const MAX_INPUT_HEIGHT = 120
/** Long enough that an instant command never pulls the caret out of the composer. */
const PTY_FOCUS_DELAY_MS = 120
const COMPLETION_DEBOUNCE_MS = 80
/** History rows shown above completions when both are present. */
const HISTORY_ROWS_WITH_COMPLETIONS = 3

type Row =
  | { type: 'history'; entry: CommandHistoryEntry }
  | { type: 'completion'; completion: Completion }

function relativeTime(timestamp: number): string {
  const delta = Date.now() - timestamp
  if (delta < 60_000) return 'now'
  if (delta < 3_600_000) return `${Math.floor(delta / 60_000)}m`
  if (delta < 86_400_000) return `${Math.floor(delta / 3_600_000)}h`
  return `${Math.floor(delta / 86_400_000)}d`
}

function RowIcon({ row }: { row: Row }) {
  const cls = 'shrink-0 text-gray-500'
  if (row.type === 'history') return <History size={11} className={cls} />
  switch (row.completion.kind) {
    case 'command':
      return <Terminal size={11} className={cls} />
    case 'subcommand':
      return <CornerDownRight size={11} className={cls} />
    case 'flag':
      return <SlidersHorizontal size={10} className={cls} />
    case 'branch':
      return <GitBranch size={11} className={cls} />
    case 'script':
      return <Play size={10} className={cls} />
    case 'path':
      return row.completion.continues ? (
        <Folder size={11} className={cls} />
      ) : (
        <File size={11} className={cls} />
      )
  }
}

/** The input minus the token under the cursor (token = trailing non-space run). */
function splitCurrentToken(value: string): { prefix: string; token: string } {
  const match = value.match(/[^\s]*$/)
  const token = match?.[0] ?? ''
  return { prefix: value.slice(0, value.length - token.length), token }
}

/** Longest common prefix of completion inserts that extends the current token. */
function longestCommonPrefix(inserts: string[]): string {
  if (inserts.length === 0) return ''
  let lcp = inserts[0]
  for (const s of inserts.slice(1)) {
    let i = 0
    while (i < lcp.length && i < s.length && lcp[i] === s[i]) i++
    lcp = lcp.slice(0, i)
    if (!lcp) break
  }
  return lcp
}

/**
 * Intent bar: the input editor for shell sessions, docked under the terminal.
 * Text is composed locally and written to the pty in one shot on submit, so
 * the terminal never sees half-typed input. The dropdown blends command
 * history with token completions (commands, subcommands, flags, paths,
 * branches, package scripts) resolved against the session's live working
 * directory. Agent sessions render nothing — their TUI owns input.
 *
 * Keys: Enter submits typed text or a highlighted history row; on a
 * highlighted completion it inserts without running. Shift+Enter inserts a
 * newline, Escape returns focus to the terminal. Tab fills the longest
 * common prefix, then inserts the highlighted (or first) completion.
 */
// Interrupts, forwarded while a command runs. Ctrl, never Cmd — on macOS Cmd+C
// is copy. Not a key table: xterm encodes the rest once it has focus.
const CONTROL_CHORDS: Record<string, string> = {
  c: '\x03', // SIGINT
  d: '\x04', // EOF
  z: '\x1a', // SIGTSTP
  '\\': '\x1c' // SIGQUIT
}

export function IntentBar({ terminalId, compact, indentPx = 16 }: Props) {
  const session = useAppStore((s) => s.terminals.get(terminalId)?.session)
  const textareaRef = useRef<HTMLTextAreaElement>(null)
  const listRef = useRef<HTMLDivElement>(null)
  const boxRef = useRef<HTMLDivElement>(null)
  // The terminal renders in a fixed overlay layer (z-45), so the menu must be
  // a body-level portal above it — an in-card absolute element would be
  // painted underneath the terminal.
  const [menuPos, setMenuPos] = useState<{ left: number; width: number; bottom: number } | null>(
    null
  )
  // Comes back after a reload or a reboot, the way an editor's unsaved edit does.
  const [value, setValue] = useState(() => readIntentDraft(terminalId)?.text ?? '')
  const [isFocused, setIsFocused] = useState(false)
  const [history, setHistory] = useState<CommandHistoryEntry[]>([])
  const [completions, setCompletions] = useState<Completion[]>([])
  const [isOpen, setIsOpen] = useState(false)
  // -1 = nothing highlighted: Enter submits the typed text, never a suggestion.
  const [highlightIndex, setHighlightIndex] = useState(-1)
  const completionReq = useRef(0)
  const debounceTimer = useRef<ReturnType<typeof setTimeout> | null>(null)

  // Runnable names, used to tell a command from a prompt. Empty until the
  // executable list resolves, which keeps everything in shell mode until we
  // actually know — see resolveIntentMode.
  const [knownCommands, setKnownCommands] = useState<ReadonlySet<string>>(new Set())
  const [pinnedMode, setPinnedMode] = useState<IntentMode | null>(null)
  const isLaunching = useRef(false)

  const defaultAgent = useAppStore((s) => s.config?.defaults.defaultAgent)
  const [agent, setAgent] = useState<AiAgentType>(() => getPreferredAgent(defaultAgent ?? 'claude'))
  const { status: installStatus } = useAgentInstallStatus()

  const isShell = isShellSession(session?.agentType)
  const projectPath = session?.projectPath
  const cwd = session?.shellCwd ?? session?.worktreePath ?? session?.projectPath ?? null
  const repoPath = session?.worktreePath ?? session?.projectPath ?? null
  // Completions need the RPC surface (absent in older preloads/tests).
  const completionsEnabled = typeof window.api?.listShellExecutables === 'function'

  // Subscribed, not read in a handler: the bar has to disappear when a command
  // takes the keyboard, which is a render concern.
  const inputState = useSyncExternalStore(
    useCallback((cb: () => void) => onCommandBlocksChange(terminalId, cb), [terminalId]),
    useCallback(() => getShellInputState(terminalId), [terminalId])
  )
  const isMobile = useIsMobile()

  // Whether the running command has lasted long enough to own the pane. Seeded
  // true so arriving at a card mid-command shows no bar rather than a flash.
  const [settled, setSettled] = useState(() => inputState === 'running')
  // Mirrors of focus and typed text, read by the handover below after the
  // textarea may already have been removed. State would be stale in that closure.
  const hadFocusRef = useRef(false)
  const draftRef = useRef('')
  useEffect(() => {
    draftRef.current = value
    writeIntentDraft(terminalId, value)
  }, [value, terminalId])
  const [seenState, setSeenState] = useState(inputState)
  if (seenState !== inputState) {
    // Adjusted during render, which React sanctions: an effect writing this
    // would render once with the previous command's answer before correcting.
    setSeenState(inputState)
    setSettled(false)
  }

  useEffect(() => {
    // Read from a ref, not the DOM: the alternate screen hides the composer in
    // the same render that reports it, so by now the textarea may already be
    // gone — and asking a detached node whether it had focus always says no.
    const yieldKeyboard = (): void => {
      // A half-typed command means the keyboard is still being used. Moving it
      // would split the rest of the word into the pty's line buffer.
      if (hadFocusRef.current && !draftRef.current) focusTerminal(terminalId)
    }
    // A full-screen program is never transient; only a command is worth waiting out.
    if (inputState === 'altScreen') {
      yieldKeyboard()
      return
    }
    if (inputState !== 'running') return
    const timer = setTimeout(() => {
      yieldKeyboard()
      setSettled(true)
    }, PTY_FOCUS_DELAY_MS)
    return () => clearTimeout(timer)
  }, [inputState, terminalId])

  // The composer belongs to the prompt; while a command owns the terminal there
  // is nothing for it to do. 'unknown' keeps it — no integration, no signal.
  //
  // Never on mobile: focus cannot move there, so hiding would leave the phone
  // with no input at all rather than with the terminal's.
  const composerHidden =
    !isMobile && (inputState === 'altScreen' || (inputState === 'running' && settled))

  const inferredMode = useMemo(
    () => resolveIntentMode(value, knownCommands),
    [value, knownCommands]
  )
  const mode = pinnedMode ?? inferredMode
  const isPrompt = mode === 'prompt'
  const kind: HistoryKind = isPrompt ? 'agent' : 'shell'

  const rows: Row[] = useMemo(() => {
    const historyCap = completions.length > 0 ? HISTORY_ROWS_WITH_COMPLETIONS : 8
    return [
      ...history.slice(0, historyCap).map((entry): Row => ({ type: 'history', entry })),
      ...completions.map((completion): Row => ({ type: 'completion', completion }))
    ]
  }, [history, completions])

  const ghost = useMemo(() => {
    if (isOpen && highlightIndex >= 0) return null
    return ghostSuggestion(value, kind, projectPath)
  }, [value, kind, projectPath, isOpen, highlightIndex])

  const closeDropdown = useCallback(() => {
    setIsOpen(false)
    setHighlightIndex(-1)
    setCompletions([])
    completionReq.current++
    if (debounceTimer.current) clearTimeout(debounceTimer.current)
  }, [])

  const fetchCompletions = useCallback(
    (input: string) => {
      // Prose has no tokens to complete; history still supplies ghost text.
      if (!completionsEnabled || isPrompt || !input.trim() || input.includes('\n')) {
        setCompletions([])
        return
      }
      const req = ++completionReq.current
      if (debounceTimer.current) clearTimeout(debounceTimer.current)
      debounceTimer.current = setTimeout(() => {
        getCompletions(input, defaultSources(cwd, repoPath))
          .then((result) => {
            // Discard results that arrive after the input moved on.
            if (completionReq.current !== req) return
            setCompletions(result)
            if (result.length > 0) setIsOpen(true)
          })
          .catch(() => {})
      }, COMPLETION_DEBOUNCE_MS)
    },
    [completionsEnabled, isPrompt, cwd, repoPath]
  )

  const openDropdown = useCallback(
    (query: string, highlightFirst: boolean) => {
      const results = searchHistory(query, kind, projectPath)
      setHistory(results)
      setIsOpen(results.length > 0)
      setHighlightIndex(highlightFirst && results.length > 0 ? 0 : -1)
      return results.length > 0
    },
    [kind, projectPath]
  )

  const autoGrow = useCallback(() => {
    const el = textareaRef.current
    if (!el) return
    el.style.height = 'auto'
    el.style.height = `${Math.min(el.scrollHeight, MAX_INPUT_HEIGHT)}px`
  }, [])

  useEffect(() => {
    autoGrow()
  }, [value, autoGrow])

  useEffect(() => {
    if (highlightIndex < 0 || !listRef.current) return
    const item = listRef.current.children[highlightIndex] as HTMLElement | undefined
    item?.scrollIntoView?.({ block: 'nearest' })
  }, [highlightIndex])

  useEffect(() => {
    return () => {
      if (debounceTimer.current) clearTimeout(debounceTimer.current)
    }
  }, [])

  // Expose the input for keystroke redirection from the raw terminal. A layout
  // effect because the composer remounts when a command ends: a passive one
  // leaves a window where the box is on screen but unregistered, and the first
  // keystroke goes to the pty instead.
  useLayoutEffect(() => {
    const el = textareaRef.current
    if (!el || !isShell) return
    return registerIntentBarInput(terminalId, el)
  }, [terminalId, isShell, composerHidden])

  // Runnable names for mode resolution. The executable list is cached at
  // module level for five minutes, so this is nearly free per mount.
  useEffect(() => {
    if (!isShell || !completionsEnabled) return
    let cancelled = false
    defaultSources(cwd, repoPath)
      .listExecutables()
      .then((executables) => {
        if (cancelled) return
        setKnownCommands(new Set([...executables, ...SHELL_BUILTINS, ...outlineNames()]))
      })
      .catch(() => {})
    return () => {
      cancelled = true
    }
  }, [isShell, completionsEnabled, cwd, repoPath])

  const menuVisible = isOpen && rows.length > 0

  useLayoutEffect(() => {
    if (!menuVisible) return
    const update = (): void => {
      const rect = boxRef.current?.getBoundingClientRect()
      if (!rect) return
      setMenuPos({ left: rect.left, width: rect.width, bottom: window.innerHeight - rect.top + 4 })
    }
    update()
    window.addEventListener('resize', update)
    return () => window.removeEventListener('resize', update)
  }, [menuVisible])

  const submit = useCallback(
    (text: string) => {
      const trimmed = text.replace(/\r/g, '').trim()
      if (!trimmed) return
      if (isPrompt) {
        if (isLaunching.current || !session) return
        isLaunching.current = true
        void launchAgentFromShell(session, agent, trimmed).finally(() => {
          isLaunching.current = false
        })
      } else {
        // paste() normalizes newlines and honors bracketed-paste mode, so
        // multiline text lands as one unit; the CR then submits it.
        pasteToTerminal(terminalId, trimmed)
        window.api.writeTerminal(terminalId, '\r')
        scrollToBottom(terminalId)
      }
      recordCommand(trimmed, kind, projectPath)
      setValue('')
      // A pin lasts one submission.
      setPinnedMode(null)
      closeDropdown()
    },
    [terminalId, kind, projectPath, closeDropdown, isPrompt, session, agent]
  )

  const refreshForValue = useCallback(
    (next: string) => {
      const query = next.trim()
      if (!query || next.includes('\n')) {
        closeDropdown()
        return
      }
      openDropdown(query, false)
      fetchCompletions(next)
    },
    [openDropdown, fetchCompletions, closeDropdown]
  )

  const insertText = useCallback(
    (text: string, refresh: boolean) => {
      setValue(text)
      setHighlightIndex(-1)
      if (refresh) {
        refreshForValue(text)
      } else {
        closeDropdown()
      }
      textareaRef.current?.focus()
    },
    [refreshForValue, closeDropdown]
  )

  /** Replace the current token; directories keep the menu open for drilling. */
  const applyCompletion = useCallback(
    (completion: Completion) => {
      const { prefix } = splitCurrentToken(value)
      const next = prefix + completion.insert + (completion.continues ? '' : ' ')
      insertText(next, true)
    },
    [value, insertText]
  )

  const applyRow = useCallback(
    (row: Row) => {
      if (row.type === 'history') {
        insertText(row.entry.text, false)
      } else {
        applyCompletion(row.completion)
      }
    },
    [insertText, applyCompletion]
  )

  const handleChange = useCallback(
    (next: string) => {
      setValue(next)
      // Clearing the field releases an override, so the bar never stays
      // pinned to a mode the user cannot see a reason for.
      if (!next.trim()) setPinnedMode(null)
      refreshForValue(next)
    },
    [refreshForValue]
  )

  const toggleMode = useCallback(() => {
    setPinnedMode(mode === 'shell' ? 'prompt' : 'shell')
  }, [mode])

  const handleAgentChange = useCallback((next: LaunchAgentType | null) => {
    // The picker can also yield null and the task sentinel; neither makes
    // sense here, where the agent is launched immediately.
    if (!next || next === 'fromTask') return
    setAgent(next)
    setPreferredAgent(next)
  }, [])

  const handleKeyDown = useCallback(
    (e: KeyboardEvent<HTMLTextAreaElement>) => {
      // An input method mid-word owns every key; swallowing one breaks it.
      if (e.nativeEvent.isComposing) return

      // Shift excluded: Ctrl+Shift+C is the terminal's copy chord. Anything but
      // 'prompt', so vim and uninstrumented shells can be interrupted too.
      if (e.ctrlKey && !e.metaKey && !e.altKey && !e.shiftKey) {
        const chord = CONTROL_CHORDS[e.key.toLowerCase()]
        const target = e.currentTarget
        const hasSelection = target.selectionStart !== target.selectionEnd
        // Copying out of the composer still wins, as it does in the terminal.
        if (chord && !(e.key.toLowerCase() === 'c' && hasSelection)) {
          if (!isAtPrompt(getShellInputState(terminalId))) {
            e.preventDefault()
            window.api.writeTerminal(terminalId, chord)
            return
          }
        }
      }

      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 'i') {
        e.preventDefault()
        toggleMode()
        return
      }

      if (e.key === 'Enter' && !e.shiftKey) {
        e.preventDefault()
        const row = isOpen && highlightIndex >= 0 ? rows[highlightIndex] : undefined
        if (row?.type === 'history') {
          submit(row.entry.text)
        } else if (row?.type === 'completion') {
          // A completion is a partial command — insert it, never run it.
          applyCompletion(row.completion)
        } else {
          submit(value)
        }
        return
      }

      if (e.key === 'Escape') {
        e.preventDefault()
        if (isOpen) {
          closeDropdown()
        } else if (pinnedMode) {
          // Release the override first — Escape should undo the last thing
          // you did, not jump straight back to the terminal.
          setPinnedMode(null)
        } else {
          textareaRef.current?.blur()
          focusTerminal(terminalId)
        }
        return
      }

      if (e.key === 'ArrowUp') {
        if (value.includes('\n')) return // multiline text: let the caret move
        e.preventDefault()
        if (!isOpen) {
          openDropdown(value.trim(), true)
        } else {
          setHighlightIndex((prev) => (prev < 0 ? 0 : Math.max(prev - 1, 0)))
        }
        return
      }

      if (e.key === 'ArrowDown') {
        if (!isOpen) return
        e.preventDefault()
        setHighlightIndex((prev) => Math.min(prev + 1, rows.length - 1))
        return
      }

      if (e.key === 'Tab') {
        if (isOpen && highlightIndex >= 0 && rows[highlightIndex]) {
          e.preventDefault()
          applyRow(rows[highlightIndex])
          return
        }
        if (completions.length > 0) {
          e.preventDefault()
          // Fill the longest common prefix first (shell-style); if it can't
          // extend the token, take the first completion.
          const { token } = splitCurrentToken(value)
          const lcp = longestCommonPrefix(completions.map((c) => c.insert))
          if (lcp.length > token.length && completions.length > 1) {
            const { prefix } = splitCurrentToken(value)
            insertText(prefix + lcp, true)
          } else {
            applyCompletion(completions[0])
          }
          return
        }
        if (ghost) {
          e.preventDefault()
          insertText(ghost, false)
        }
        return
      }

      if (e.key === 'ArrowRight' && ghost) {
        const el = textareaRef.current
        if (el && el.selectionStart === value.length && el.selectionEnd === value.length) {
          e.preventDefault()
          insertText(ghost, false)
        }
      }
    },
    [
      isOpen,
      highlightIndex,
      rows,
      completions,
      value,
      ghost,
      terminalId,
      submit,
      closeDropdown,
      openDropdown,
      applyRow,
      applyCompletion,
      insertText,
      pinnedMode,
      toggleMode
    ]
  )

  // Agent sessions keep their own TUI input; the composer is shell-only.
  if (!session || !isShell) return null

  // The running command gets the whole pane. The effect above has already handed
  // the keyboard over by the time this renders nothing.
  if (composerHidden) return null

  // The accessible name always states the mode the input is actually in, so
  // it is unambiguous once there is something to classify.
  const label = isPrompt ? 'Describe a task' : 'Type a command'
  // An empty, unpinned input has nothing to classify yet — it resolves to
  // command mode by default, but either kind of input is accepted, and the
  // hint is the only place that says so. Once a mode is pinned the hint names
  // just that one, because the choice has been made.
  const placeholder = pinnedMode || value ? label : 'Type a command or send a prompt for the agent'
  const ghostRemainder = ghost && !value.includes('\n') ? ghost.slice(value.length) : ''
  // The status bar below already shows the branch; surface the working
  // directory here instead — that's what a prompt would tell you.
  const cwdLabel = (cwd ? getDisplayPathBasename(cwd) : undefined) ?? session.projectName
  // Mode is carried by typeface and glyph, never by colour — colour stays
  // reserved for status. Shared by the textarea and the ghost overlay so the
  // ghost remainder cannot drift out of alignment.
  const inputTextClass = isPrompt
    ? 'font-sans italic text-[13px] leading-[19px]'
    : 'font-mono text-[13px] leading-[19px]'

  return (
    <div className="relative shrink-0" style={{ background: 'var(--color-surface-sunken)' }}>
      {menuVisible &&
        menuPos &&
        createPortal(
          <div
            className="fixed z-[60] rounded-[3px] border border-white/[0.07]
                       bg-surface-overlay overflow-hidden"
            style={{ left: menuPos.left, width: menuPos.width, bottom: menuPos.bottom }}
          >
            <div
              ref={listRef}
              role="listbox"
              aria-label="Suggestions"
              className="max-h-[192px] overflow-y-auto py-0.5"
            >
              {rows.map((row, i) => {
                const key =
                  row.type === 'history'
                    ? `h-${row.entry.text}-${row.entry.timestamp}`
                    : `c-${row.completion.kind}-${row.completion.insert}`
                const label = row.type === 'history' ? row.entry.text : row.completion.label
                return (
                  <button
                    key={key}
                    type="button"
                    onPointerDown={(e) => {
                      // Keep focus in the textarea while clicking a row.
                      e.preventDefault()
                      e.stopPropagation()
                      if (row.type === 'history') {
                        submit(row.entry.text)
                      } else {
                        applyCompletion(row.completion)
                      }
                    }}
                    onPointerEnter={() => setHighlightIndex(i)}
                    className={`w-full flex items-center gap-2 px-2.5 py-1 text-left font-mono text-[12px]
                              ${i === highlightIndex ? 'bg-white/[0.08] text-gray-100' : 'text-gray-400'}`}
                  >
                    <RowIcon row={row} />
                    <span className="flex-1 truncate">{label}</span>
                    {i === highlightIndex ? (
                      row.type === 'history' ? (
                        <CornerDownLeft size={11} className="shrink-0 text-gray-500" />
                      ) : (
                        <span className="shrink-0 text-[10px] text-gray-500">⇥</span>
                      )
                    ) : row.type === 'history' ? (
                      <span className="shrink-0 text-[10px] text-gray-600">
                        {relativeTime(row.entry.timestamp)}
                      </span>
                    ) : row.completion.detail ? (
                      <span className="shrink-0 text-[10px] text-gray-600">
                        {row.completion.detail}
                      </span>
                    ) : null}
                  </button>
                )
              })}
            </div>
          </div>,
          document.body
        )}

      {/* No border, no radius, no raised fill: the input sits on the terminal's
          own ground and lines up with its text column, so it reads as the next
          line of the session rather than a separate control. */}
      <div
        ref={boxRef}
        className="border-t border-white/[0.05] focus-within:bg-white/[0.02] transition-colors"
        style={{ paddingLeft: indentPx }}
      >
        <div className={`flex items-start gap-2 pr-2 ${compact ? 'py-1' : 'pt-1.5 pb-1'}`}>
          <button
            type="button"
            onPointerDown={(e) => e.preventDefault()}
            onClick={toggleMode}
            aria-label={isPrompt ? 'Prompt mode' : 'Command mode'}
            title={`${isPrompt ? 'Prompt' : 'Command'} — ⌘I to switch`}
            className={`shrink-0 font-mono text-[13px] leading-[19px] transition-colors
                        ${isFocused ? 'text-gray-300' : 'text-gray-600'}
                        ${pinnedMode ? 'border-b border-white/25' : ''}`}
          >
            {/* The same caret the input shows in either mode, so the column is
                continuous from the last command into the composer. */}
            {isPrompt ? '◇' : '❯'}
          </button>
          <div className="relative flex-1 min-w-0">
            {ghostRemainder && (
              <div
                aria-hidden
                className={`absolute inset-0 pointer-events-none ${inputTextClass}
                           whitespace-pre-wrap break-words overflow-hidden`}
              >
                <span className="invisible">{value}</span>
                <span className="text-gray-600">{ghostRemainder}</span>
              </div>
            )}
            <textarea
              ref={textareaRef}
              rows={1}
              value={value}
              spellCheck={false}
              autoCapitalize="off"
              autoCorrect="off"
              placeholder={placeholder}
              aria-label={label}
              onChange={(e) => handleChange(e.target.value)}
              onKeyDown={handleKeyDown}
              onFocus={() => {
                hadFocusRef.current = true
                setIsFocused(true)
              }}
              onBlur={() => {
                hadFocusRef.current = false
                setIsFocused(false)
                closeDropdown()
              }}
              data-no-focus-ring
              className={`block w-full resize-none bg-transparent ${inputTextClass}
                         text-gray-200 placeholder:text-gray-700
                         focus:outline-none focus-visible:outline-none
                         whitespace-pre-wrap break-words`}
              style={{ maxHeight: MAX_INPUT_HEIGHT }}
            />
          </div>
          {/* On the input row, not in the hint strip: which agent a prompt
              starts is part of the action, so it stays visible rather than
              fading with the shortcut hints. */}
          {isPrompt && (
            <div className="shrink-0 -mt-[1px]">
              <AgentPicker
                currentAgent={agent}
                onChange={handleAgentChange}
                installStatus={installStatus}
                variant="compact"
              />
            </div>
          )}
        </div>

        {!compact && (
          // Hints are for the moment you are typing, so they fade back when
          // the input is idle rather than sitting there as permanent chrome.
          <div
            className={`flex items-center gap-2 pr-2 pb-1 text-[10px] text-gray-700
                        transition-opacity duration-150
                        ${isFocused || value ? 'opacity-100' : 'opacity-0'}`}
          >
            <span className="flex items-center gap-1 min-w-0" title={cwd ?? undefined}>
              <Folder size={9} className="shrink-0" strokeWidth={1.5} />
              <span className="font-mono truncate max-w-[200px]">{cwdLabel}</span>
            </span>
            <div className="flex-1" />
            <span>↑ history</span>
            {isPrompt ? (
              <>
                <span>↵ launch</span>
                <span>⌘I command</span>
              </>
            ) : (
              <>
                <span>⇥ complete</span>
                <span>↵ run</span>
              </>
            )}
          </div>
        )}
      </div>
    </div>
  )
}
