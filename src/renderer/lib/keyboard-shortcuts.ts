export type ShortcutCategory = 'navigation' | 'sessions' | 'view' | 'filter'

export interface ShortcutDef {
  id: string
  key: string
  meta?: boolean
  shift?: boolean
  display: string
  requireNoFocus?: boolean
  category?: ShortcutCategory
  description?: string
}

const isMac = navigator.platform.toUpperCase().includes('MAC')
const MOD = isMac ? '\u2318' : 'Ctrl+'
const ALT = isMac ? '\u2325' : 'Alt+'

export const SHORTCUTS: ShortcutDef[] = [
  // Navigation
  {
    id: 'focus-next',
    key: ']',
    meta: true,
    display: `${MOD}]`,
    category: 'navigation',
    description: 'Next card'
  },
  {
    id: 'focus-prev',
    key: '[',
    meta: true,
    display: `${MOD}[`,
    category: 'navigation',
    description: 'Previous card'
  },
  {
    id: 'escape',
    key: 'Escape',
    display: 'Esc',
    category: 'navigation',
    description: 'Close / deselect'
  },

  // Sessions
  {
    id: 'new-session',
    key: 'n',
    meta: true,
    display: `${MOD}N`,
    category: 'sessions',
    description: 'New session'
  },
  { id: 'rename', key: 'F2', display: 'F2', category: 'sessions', description: 'Rename session' },

  // View
  {
    id: 'command-palette',
    key: 'k',
    meta: true,
    display: `${MOD}K`,
    category: 'view',
    description: 'Command palette'
  },
  {
    id: 'shortcuts-panel',
    key: '/',
    meta: true,
    display: `${MOD}/`,
    category: 'view',
    description: 'Keyboard shortcuts'
  },
  {
    id: 'settings',
    key: ',',
    meta: true,
    display: `${MOD},`,
    category: 'view',
    description: 'Settings'
  },
  {
    id: 'toggle-sidebar',
    key: 'b',
    meta: true,
    display: `${MOD}B`,
    category: 'view',
    description: 'Toggle sidebar'
  },
  {
    id: 'new-terminal',
    key: '`',
    display: 'Ctrl+`',
    category: 'view',
    description: 'New terminal session'
  },
  {
    id: 'view-sessions',
    key: 's',
    meta: true,
    display: `${MOD}S`,
    category: 'view',
    description: 'Sessions view'
  },
  {
    id: 'view-tasks',
    key: 't',
    meta: true,
    display: `${MOD}T`,
    category: 'view',
    description: 'Tasks view'
  },
  {
    id: 'view-workflows',
    key: 'w',
    meta: true,
    shift: true,
    display: `${MOD}⇧W`,
    category: 'view',
    description: 'Workflows view'
  },
  {
    id: 'view-options',
    key: 'j',
    meta: true,
    display: `${MOD}J`,
    category: 'view',
    description: 'View options'
  },

  // Navigation — card jump
  {
    id: 'jump-to-card',
    key: '1-9',
    meta: true,
    display: `${MOD}1-9`,
    category: 'navigation',
    description: 'Jump to card by position'
  },

  // Filters
  {
    id: 'filter-all',
    key: '1',
    display: `${ALT}1`,
    category: 'filter',
    description: 'Show all'
  },
  {
    id: 'filter-running',
    key: '2',
    display: `${ALT}2`,
    category: 'filter',
    description: 'Show running'
  },
  {
    id: 'filter-waiting',
    key: '3',
    display: `${ALT}3`,
    category: 'filter',
    description: 'Show waiting'
  },
  {
    id: 'filter-idle',
    key: '4',
    display: `${ALT}4`,
    category: 'filter',
    description: 'Show idle'
  },
  {
    id: 'filter-error',
    key: '5',
    display: `${ALT}5`,
    category: 'filter',
    description: 'Show errors'
  },
  // Workflow canvas
  {
    id: 'workflow-fit',
    key: '1',
    display: '1',
    requireNoFocus: true,
    category: 'view',
    description: 'Fit the whole workflow on the canvas'
  }
]

export const SHORTCUT_CATEGORIES: { key: ShortcutCategory; label: string }[] = [
  { key: 'navigation', label: 'Navigation' },
  { key: 'sessions', label: 'Sessions' },
  { key: 'view', label: 'View' },
  { key: 'filter', label: 'Filters' }
]

export function getShortcut(id: string): ShortcutDef | undefined {
  return SHORTCUTS.find((s) => s.id === id)
}
