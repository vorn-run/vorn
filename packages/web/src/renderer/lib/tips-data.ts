const isMac = navigator.platform.toUpperCase().includes('MAC')
const MOD = isMac ? '\u2318' : 'Ctrl+'
const ALT = isMac ? '\u2325' : 'Alt+'

export interface Tip {
  text: string
  shortcut?: string
}

export const TIPS: Tip[] = [
  {
    text: 'Open the command palette to quickly launch agents or switch sessions',
    shortcut: `${MOD}K`
  },
  { text: 'Toggle the sidebar to focus on your grid', shortcut: `${MOD}B` },
  { text: 'View all keyboard shortcuts', shortcut: `${MOD}/` },
  { text: 'Cycle between agent cards', shortcut: `${MOD}] / ${MOD}[` },
  { text: 'Double-click a card title to rename it inline' },
  { text: 'Create a worktree to give each agent an isolated working directory' },
  { text: 'Jump directly to any card by its position', shortcut: `${MOD}1\u2013${MOD}9` },
  {
    text: 'Use status filters to focus on running or waiting agents',
    shortcut: `${ALT}1\u2013${ALT}5`
  },
  { text: 'Double-click the empty grid area to quick-launch a session with your default agent' },
  { text: 'Right-click a card for quick actions: rename, launch new session, or close' },
  { text: 'Right-click the empty grid area to launch a new session or worktree session' },
  { text: 'Launch a worktree session from the Worktrees section in the sidebar' },
  { text: 'Set up Workflows in the sidebar to launch multi-agent setups with one click' },
  { text: 'Click the diff badge on a card to review all changes an agent has made' },
  { text: 'Drag cards to reorder them when in manual sort mode' },
  { text: 'Add remote hosts in Settings to run agents on other machines via SSH' },
  { text: 'Resume previous sessions from the clock icon in the top toolbar' },
  {
    text: 'Open Settings to customize font size, default agent, and notifications',
    shortcut: `${MOD},`
  }
]

export function getRandomTips(count: number): Tip[] {
  const shuffled = [...TIPS].sort(() => Math.random() - 0.5)
  return shuffled.slice(0, count)
}
