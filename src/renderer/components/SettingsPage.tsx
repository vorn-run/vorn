import { motion } from 'framer-motion'
import { isElectron } from '../lib/platform'
import { useAppStore } from '../stores'
import { SettingsCategory } from '../stores/types'
import { SectionHeader } from './settings/SectionHeader'
import { AppearanceSettings } from './settings/AppearanceSettings'
import { GeneralSettings } from './settings/GeneralSettings'
import { UpdatesSettings } from './settings/UpdatesSettings'
import { NotificationSettings } from './settings/NotificationSettings'
import { WorktreeSettings } from './settings/WorktreeSettings'
import { AgentSettings } from './settings/AgentSettings'
import { SSHSettings } from './settings/SSHSettings'
import { McpSettings } from './settings/McpSettings'
import { ConnectorSettings } from './settings/ConnectorSettings'
import { NetworkSettings } from './settings/NetworkSettings'
import { AboutSettings } from './settings/AboutSettings'

interface SidebarSection {
  header: string
  items: { key: SettingsCategory; label: string; icon: React.ReactNode }[]
}

const SIDEBAR_SECTIONS: SidebarSection[] = [
  {
    header: 'APP SETTINGS',
    items: [
      {
        key: 'appearance',
        label: 'Appearance',
        icon: (
          <svg
            width="16"
            height="16"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            strokeWidth="1.5"
          >
            <circle cx="12" cy="12" r="5" />
            <path d="M12 1v2M12 21v2M4.22 4.22l1.42 1.42M18.36 18.36l1.42 1.42M1 12h2M21 12h2M4.22 19.78l1.42-1.42M18.36 5.64l1.42-1.42" />
          </svg>
        )
      },
      {
        key: 'general',
        label: 'General',
        icon: (
          <svg
            width="16"
            height="16"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            strokeWidth="1.5"
          >
            <circle cx="12" cy="12" r="3" />
            <path d="M19.4 15a1.65 1.65 0 00.33 1.82l.06.06a2 2 0 010 2.83 2 2 0 01-2.83 0l-.06-.06a1.65 1.65 0 00-1.82-.33 1.65 1.65 0 00-1 1.51V21a2 2 0 01-4 0v-.09A1.65 1.65 0 009 19.4a1.65 1.65 0 00-1.82.33l-.06.06a2 2 0 01-2.83 0 2 2 0 010-2.83l.06-.06A1.65 1.65 0 004.68 15a1.65 1.65 0 00-1.51-1H3a2 2 0 010-4h.09A1.65 1.65 0 004.6 9a1.65 1.65 0 00-.33-1.82l-.06-.06a2 2 0 112.83-2.83l.06.06a1.65 1.65 0 001.82.33H9a1.65 1.65 0 001-1.51V3a2 2 0 014 0v.09a1.65 1.65 0 001 1.51 1.65 1.65 0 001.82-.33l.06-.06a2 2 0 112.83 2.83l-.06.06A1.65 1.65 0 0019.4 9a1.65 1.65 0 001.51 1H21a2 2 0 010 4h-.09a1.65 1.65 0 00-1.51 1z" />
          </svg>
        )
      },
      {
        key: 'updates',
        label: 'Updates',
        icon: (
          <svg
            width="16"
            height="16"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            strokeWidth="1.5"
          >
            <path d="M12 3v12" />
            <path d="M7 10l5 5 5-5" />
            <path d="M4 20h16" />
          </svg>
        )
      },
      {
        key: 'notifications',
        label: 'Notifications',
        icon: (
          <svg
            width="16"
            height="16"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            strokeWidth="1.5"
          >
            <path d="M18 8A6 6 0 006 8c0 7-3 9-3 9h18s-3-2-3-9" />
            <path d="M13.73 21a2 2 0 01-3.46 0" />
          </svg>
        )
      },
      {
        key: 'worktrees',
        label: 'Worktrees',
        icon: (
          <svg
            width="16"
            height="16"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            strokeWidth="1.5"
          >
            <circle cx="6" cy="5" r="2.5" />
            <circle cx="18" cy="5" r="2.5" />
            <circle cx="12" cy="19" r="2.5" />
            <path d="M6 7.5v3a2 2 0 002 2h8a2 2 0 002-2v-3M12 12.5v4" />
          </svg>
        )
      }
    ]
  },
  {
    header: 'CONNECTIONS',
    items: [
      {
        key: 'agents',
        label: 'Coding Agents',
        icon: (
          <svg
            width="16"
            height="16"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            strokeWidth="1.5"
          >
            <polyline points="4 17 10 11 4 5" />
            <line x1="12" y1="19" x2="20" y2="19" />
          </svg>
        )
      },
      {
        key: 'ssh',
        label: 'SSH & Hosts',
        icon: (
          <svg
            width="16"
            height="16"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            strokeWidth="1.5"
          >
            <rect x="2" y="2" width="20" height="8" rx="2" />
            <rect x="2" y="14" width="20" height="8" rx="2" />
            <circle cx="6" cy="6" r="1" fill="currentColor" />
            <circle cx="6" cy="18" r="1" fill="currentColor" />
          </svg>
        )
      },
      {
        key: 'mcp',
        label: 'MCP',
        icon: (
          <svg
            width="16"
            height="16"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            strokeWidth="1.5"
          >
            <path d="M12 2v4m0 12v4M2 12h4m12 0h4" />
            <circle cx="12" cy="12" r="4" />
          </svg>
        )
      },
      {
        key: 'connectors',
        label: 'Connectors',
        icon: (
          <svg
            width="16"
            height="16"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            strokeWidth="1.5"
          >
            <path d="M9 17H7A5 5 0 017 7h2" />
            <path d="M15 7h2a5 5 0 010 10h-2" />
            <line x1="8" y1="12" x2="16" y2="12" />
          </svg>
        )
      },
      {
        key: 'network',
        label: 'Remote Access',
        icon: (
          <svg
            width="16"
            height="16"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            strokeWidth="1.5"
          >
            <circle cx="12" cy="12" r="10" />
            <line x1="2" y1="12" x2="22" y2="12" />
            <path d="M12 2a15.3 15.3 0 014 10 15.3 15.3 0 01-4 10 15.3 15.3 0 01-4-10 15.3 15.3 0 014-10z" />
          </svg>
        )
      }
    ]
  }
]

export function SettingsPage() {
  const setSettingsOpen = useAppStore((s) => s.setSettingsOpen)
  const settingsCategory = useAppStore((s) => s.settingsCategory)
  const setSettingsCategory = useAppStore((s) => s.setSettingsCategory)

  return (
    <motion.div
      className="fixed inset-0 z-50 flex"
      style={{
        background: 'var(--color-surface-overlay)',
        paddingTop: 'var(--safe-top, 0px)',
        paddingRight: 'var(--safe-right, 0px)',
        paddingBottom: 'var(--safe-bottom, 0px)',
        paddingLeft: 'var(--safe-left, 0px)'
      }}
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      exit={{ opacity: 0 }}
      transition={{ duration: 0.15 }}
    >
      {/* Left sidebar */}
      <div
        className="w-56 border-r border-white/[0.06] flex flex-col shrink-0"
        style={{ background: 'var(--color-surface-sunken)' }}
      >
        {/* Header — pl-[78px] for macOS traffic light safe zone (Electron only) */}
        <div
          className={`titlebar-drag h-[52px] pr-3 flex items-center border-b border-white/[0.06] shrink-0 ${isElectron ? 'pl-[78px]' : 'pl-3'}`}
        >
          <button
            onClick={() => setSettingsOpen(false)}
            className="titlebar-no-drag flex items-center gap-2 text-sm text-gray-400 hover:text-white transition-colors"
          >
            <svg
              width="16"
              height="16"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              strokeWidth="2"
            >
              <polyline points="15 18 9 12 15 6" />
            </svg>
            Back to app
          </button>
        </div>

        {/* Sections */}
        <div className="flex-1 px-3 pt-1 overflow-auto">
          {SIDEBAR_SECTIONS.map((section) => (
            <div key={section.header}>
              <SectionHeader label={section.header} />
              <div className="space-y-0.5">
                {section.items
                  // The updater only exists in the packaged desktop app, so the
                  // web build would otherwise offer a panel of dead controls.
                  .filter((item) => item.key !== 'updates' || isElectron)
                  .map((item) => (
                    <button
                      key={item.key}
                      onClick={() => setSettingsCategory(item.key)}
                      className={`w-full text-left px-3 py-2 rounded-md text-[13px] transition-colors flex items-center gap-2.5 ${
                        settingsCategory === item.key
                          ? 'bg-white/[0.08] text-white'
                          : 'text-gray-400 hover:text-white hover:bg-white/[0.04]'
                      }`}
                    >
                      {item.icon}
                      {item.label}
                    </button>
                  ))}
              </div>
            </div>
          ))}
        </div>

        {/* About — pinned to bottom */}
        <div className="px-3 pb-4 pt-2 border-t border-white/[0.06]">
          <button
            onClick={() => setSettingsCategory('about')}
            className={`w-full text-left px-3 py-2 rounded-md text-[13px] transition-colors flex items-center gap-2.5 ${
              settingsCategory === 'about'
                ? 'bg-white/[0.08] text-white'
                : 'text-gray-500 hover:text-white hover:bg-white/[0.04]'
            }`}
          >
            <svg
              width="16"
              height="16"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              strokeWidth="1.5"
            >
              <circle cx="12" cy="12" r="10" />
              <line x1="12" y1="16" x2="12" y2="12" />
              <line x1="12" y1="8" x2="12.01" y2="8" />
            </svg>
            About
          </button>
        </div>
      </div>

      {/* Content area */}
      <div className="flex-1 overflow-auto">
        <div className="max-w-2xl mx-auto px-8 py-8">
          {settingsCategory === 'appearance' && <AppearanceSettings />}
          {settingsCategory === 'general' && <GeneralSettings />}
          {settingsCategory === 'updates' && <UpdatesSettings />}
          {settingsCategory === 'notifications' && <NotificationSettings />}
          {settingsCategory === 'worktrees' && <WorktreeSettings />}
          {settingsCategory === 'agents' && <AgentSettings />}
          {settingsCategory === 'ssh' && <SSHSettings />}
          {settingsCategory === 'mcp' && <McpSettings />}
          {settingsCategory === 'connectors' && <ConnectorSettings />}
          {settingsCategory === 'network' && <NetworkSettings />}
          {settingsCategory === 'about' && <AboutSettings />}
        </div>
      </div>
    </motion.div>
  )
}
