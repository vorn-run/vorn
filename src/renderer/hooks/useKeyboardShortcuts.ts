import { useEffect } from 'react'
import { useAppStore } from '../stores'
import { StatusFilter } from '../stores/types'
import { resolveActiveProject, createShellInProject } from '../lib/session-utils'

const STATUS_FILTERS: StatusFilter[] = ['all', 'running', 'waiting', 'idle', 'error']
const isMac = navigator.platform.toUpperCase().includes('MAC')
const modKey = (e: KeyboardEvent): boolean => (isMac ? e.metaKey : e.ctrlKey)

function _isInputFocused(): boolean {
  const el = document.activeElement
  if (!el) return false
  const tag = el.tagName.toLowerCase()
  return (
    tag === 'input' ||
    tag === 'textarea' ||
    tag === 'select' ||
    (el as HTMLElement).isContentEditable
  )
}

export function useKeyboardShortcuts() {
  useEffect(() => {
    const handler = (e: KeyboardEvent): void => {
      const state = useAppStore.getState()

      // Cmd+W — close focused terminal (xterm/agents capture plain Escape)
      if (modKey(e) && e.key === 'w') {
        if (state.focusedTerminalId) {
          e.preventDefault()
          state.setSelectedTerminal(state.focusedTerminalId)
          state.setFocusedTerminal(null)
          return
        }
      }

      // Escape chain — layered dismiss
      if (e.key === 'Escape') {
        if (state.isCommandPaletteOpen) {
          state.setCommandPaletteOpen(false)
          return
        }
        if (state.isOnboardingOpen) {
          state.setOnboardingOpen(false)
          return
        }
        if (state.isShortcutsPanelOpen) {
          state.setShortcutsPanelOpen(false)
          return
        }
        if (state.isSettingsOpen) {
          state.setSettingsOpen(false)
          return
        }
        if (state.maximizedPaneId) {
          state.setMaximizedPane(null)
          return
        }
        if (state.diffSidebarTerminalId) {
          state.setDiffSidebarTerminalId(null)
          return
        }
        if (state.renamingTerminalId) {
          state.setRenamingTerminalId(null)
          return
        }
        if (state.focusedTerminalId) {
          // Close overlay, preserve selection on the card that was expanded
          state.setSelectedTerminal(state.focusedTerminalId)
          state.setFocusedTerminal(null)
          return
        }
        if (state.selectedTerminalId) {
          state.setSelectedTerminal(null)
          return
        }
        return
      }

      // Cmd+K — command palette
      if (modKey(e) && e.key === 'k') {
        e.preventDefault()
        state.setCommandPaletteOpen(!state.isCommandPaletteOpen)
        return
      }

      // Cmd+/ — keyboard shortcuts panel
      if (modKey(e) && e.key === '/') {
        e.preventDefault()
        state.setShortcutsPanelOpen(!state.isShortcutsPanelOpen)
        return
      }

      // Cmd+, — settings
      if (modKey(e) && e.key === ',') {
        e.preventDefault()
        state.setSettingsOpen(true)
        return
      }

      // Cmd+N — new session
      if (modKey(e) && e.key === 'n') {
        e.preventDefault()
        state.setNewAgentDialogOpen(true)
        return
      }

      // Cmd+B — toggle sidebar
      if (modKey(e) && e.key === 'b') {
        e.preventDefault()
        state.toggleSidebar()
        return
      }

      if (e.ctrlKey && e.key === '`') {
        e.preventDefault()
        const project = resolveActiveProject()
        void createShellInProject(project?.path)
        return
      }

      // Cmd+] — next terminal
      if (modKey(e) && e.key === ']') {
        e.preventDefault()
        const ids = state.visibleTerminalIds
        const layoutMode = state.config?.defaults?.layoutMode ?? 'grid'
        if (state.focusedTerminalId) {
          // Focused mode spans the whole project (across worktrees), not just
          // the worktree-filtered grid view.
          const focusable = state.focusableTerminalIds
          if (focusable.length === 0) return
          const current = focusable.indexOf(state.focusedTerminalId)
          const next = current === -1 ? 0 : (current + 1) % focusable.length
          state.setFocusedTerminal(focusable[next])
        } else if (ids.length === 0) {
          return
        } else if (layoutMode === 'tabs') {
          const currentTab = state.activeTabId
          const current = currentTab ? ids.indexOf(currentTab) : -1
          const next = current === -1 ? 0 : (current + 1) % ids.length
          state.setActiveTabId(ids[next])
        } else {
          const currentSel = state.selectedTerminalId
          if (!currentSel) {
            state.setSelectedTerminal(ids[0])
          } else {
            const current = ids.indexOf(currentSel)
            const next = current === -1 ? 0 : (current + 1) % ids.length
            state.setSelectedTerminal(ids[next])
          }
        }
        return
      }

      // Cmd+[ — previous terminal
      if (modKey(e) && e.key === '[') {
        e.preventDefault()
        const ids = state.visibleTerminalIds
        const layoutMode = state.config?.defaults?.layoutMode ?? 'grid'
        if (state.focusedTerminalId) {
          const focusable = state.focusableTerminalIds
          if (focusable.length === 0) return
          const current = focusable.indexOf(state.focusedTerminalId)
          const prev =
            current === -1
              ? focusable.length - 1
              : (current - 1 + focusable.length) % focusable.length
          state.setFocusedTerminal(focusable[prev])
        } else if (ids.length === 0) {
          return
        } else if (layoutMode === 'tabs') {
          const currentTab = state.activeTabId
          const current = currentTab ? ids.indexOf(currentTab) : -1
          const prev = current === -1 ? ids.length - 1 : (current - 1 + ids.length) % ids.length
          state.setActiveTabId(ids[prev])
        } else {
          const currentSel = state.selectedTerminalId
          if (!currentSel) {
            state.setSelectedTerminal(ids[ids.length - 1])
          } else {
            const current = ids.indexOf(currentSel)
            const prev = current === -1 ? ids.length - 1 : (current - 1 + ids.length) % ids.length
            state.setSelectedTerminal(ids[prev])
          }
        }
        return
      }

      // Cmd+S — sessions view
      if (modKey(e) && !e.shiftKey && e.key.toLowerCase() === 's') {
        e.preventDefault()
        state.setMainViewMode('sessions')
        return
      }

      // Cmd+T — tasks view
      if (modKey(e) && !e.shiftKey && e.key.toLowerCase() === 't') {
        e.preventDefault()
        state.setMainViewMode('tasks')
        return
      }

      // Cmd+Shift+W — workflows view
      if (modKey(e) && e.shiftKey && e.key.toLowerCase() === 'w') {
        e.preventDefault()
        state.setMainViewMode('workflows')
        return
      }

      // Cmd+J — view options
      if (modKey(e) && !e.shiftKey && e.key.toLowerCase() === 'j' && !_isInputFocused()) {
        e.preventDefault()
        window.dispatchEvent(new CustomEvent('toggle-view-options'))
        return
      }

      // Cmd+1-9 — jump to card by position
      if (modKey(e) && !e.altKey && e.key >= '1' && e.key <= '9') {
        e.preventDefault()
        const index = parseInt(e.key) - 1
        const ids = state.visibleTerminalIds
        if (index < ids.length) {
          const layoutMode = state.config?.defaults?.layoutMode ?? 'grid'
          if (state.focusedTerminalId) {
            state.setFocusedTerminal(ids[index])
          } else if (layoutMode === 'tabs') {
            state.setActiveTabId(ids[index])
          } else {
            state.setSelectedTerminal(ids[index])
          }
        }
        return
      }

      // Alt+1-5 — status filters (use e.code since Alt produces special chars on Mac)
      if (e.altKey && !modKey(e) && e.code >= 'Digit1' && e.code <= 'Digit5') {
        e.preventDefault()
        const index = parseInt(e.code.charAt(5)) - 1
        if (index < STATUS_FILTERS.length) {
          state.setStatusFilter(STATUS_FILTERS[index])
        }
        return
      }

      // Cmd+O — expand selected terminal (grid mode only)
      if (modKey(e) && e.key === 'o' && !state.focusedTerminalId && state.selectedTerminalId) {
        e.preventDefault()
        const layoutMode = state.config?.defaults?.layoutMode ?? 'grid'
        if (layoutMode === 'tabs') return
        state.setFocusedTerminal(state.selectedTerminalId)
        return
      }

      // F2 — rename focused or selected terminal
      if (e.key === 'F2') {
        const targetId = state.focusedTerminalId || state.selectedTerminalId
        if (targetId) {
          e.preventDefault()
          state.setRenamingTerminalId(targetId)
          return
        }
      }
    }

    window.addEventListener('keydown', handler)
    return () => window.removeEventListener('keydown', handler)
  }, [])
}
