import { useState } from 'react'
import { useAppStore } from '../../stores'
import { AiAgentType, AgentCommandConfig } from '../../../shared/types'
import { DEFAULT_AGENT_COMMANDS } from '../../../shared/agent-defaults'
import { AGENT_LIST, AGENT_DEFINITIONS } from '../../lib/agent-definitions'
import { AgentIcon } from '../AgentIcon'
import { useAgentInstallStatus } from '../../hooks/useAgentInstallStatus'
import {
  CheckCircle,
  Copy,
  RefreshCw,
  ExternalLink,
  Terminal,
  Download,
  RotateCcw,
  ChevronDown,
  ChevronRight
} from 'lucide-react'

export function AgentSettings() {
  const config = useAppStore((s) => s.config)
  const setConfig = useAppStore((s) => s.setConfig)
  const [editingArgs, setEditingArgs] = useState<Record<string, string>>({})
  const [editingHeadlessArgs, setEditingHeadlessArgs] = useState<Record<string, string>>({})
  const [copiedAgent, setCopiedAgent] = useState<string | null>(null)
  const [expandedNotInstalled, setExpandedNotInstalled] = useState<Record<string, boolean>>({})
  const {
    status: installStatus,
    loading: detectLoading,
    refresh: refreshDetection
  } = useAgentInstallStatus()

  if (!config) return null

  const getCommand = (agentType: AiAgentType): AgentCommandConfig => {
    return config.agentCommands?.[agentType] || DEFAULT_AGENT_COMMANDS[agentType]
  }

  const updateAgentCommand = (agentType: AiAgentType, patch: Partial<AgentCommandConfig>): void => {
    const current = getCommand(agentType)
    const updated = {
      ...config,
      agentCommands: {
        ...DEFAULT_AGENT_COMMANDS,
        ...config.agentCommands,
        [agentType]: { ...current, ...patch }
      }
    }
    window.api.saveConfig(updated)
    setConfig(updated)
  }

  const resetAgent = (agentType: AiAgentType): void => {
    const defaults = DEFAULT_AGENT_COMMANDS[agentType]
    const updated = {
      ...config,
      agentCommands: {
        ...DEFAULT_AGENT_COMMANDS,
        ...config.agentCommands,
        [agentType]: { ...defaults }
      }
    }
    window.api.saveConfig(updated)
    setConfig(updated)
    setEditingArgs((prev) => {
      const next = { ...prev }
      delete next[agentType]
      return next
    })
    setEditingHeadlessArgs((prev) => {
      const next = { ...prev }
      delete next[agentType]
      return next
    })
  }

  const getArgsString = (agentType: AiAgentType): string => {
    if (editingArgs[agentType] !== undefined) return editingArgs[agentType]
    return getCommand(agentType).args.join(' ')
  }

  const handleArgsChange = (agentType: AiAgentType, value: string): void => {
    setEditingArgs((prev) => ({ ...prev, [agentType]: value }))
  }

  const handleArgsBlur = (agentType: AiAgentType): void => {
    const value = editingArgs[agentType]
    if (value !== undefined) {
      const args = value.trim() ? value.trim().split(/\s+/) : []
      updateAgentCommand(agentType, { args })
      setEditingArgs((prev) => {
        const next = { ...prev }
        delete next[agentType]
        return next
      })
    }
  }

  const getHeadlessArgsString = (agentType: AiAgentType): string => {
    if (editingHeadlessArgs[agentType] !== undefined) return editingHeadlessArgs[agentType]
    return (getCommand(agentType).headlessArgs || []).join(' ')
  }

  const handleHeadlessArgsChange = (agentType: AiAgentType, value: string): void => {
    setEditingHeadlessArgs((prev) => ({ ...prev, [agentType]: value }))
  }

  const handleHeadlessArgsBlur = (agentType: AiAgentType): void => {
    const value = editingHeadlessArgs[agentType]
    if (value !== undefined) {
      const headlessArgs = value.trim() ? value.trim().split(/\s+/) : []
      updateAgentCommand(agentType, { headlessArgs })
      setEditingHeadlessArgs((prev) => {
        const next = { ...prev }
        delete next[agentType]
        return next
      })
    }
  }

  const isModified = (agentType: AiAgentType): boolean => {
    const current = getCommand(agentType)
    const defaults = DEFAULT_AGENT_COMMANDS[agentType]
    return (
      current.command !== defaults.command ||
      JSON.stringify(current.args) !== JSON.stringify(defaults.args) ||
      JSON.stringify(current.headlessArgs || []) !== JSON.stringify(defaults.headlessArgs || [])
    )
  }

  const copyInstallCommand = (agentType: AiAgentType): void => {
    const def = AGENT_DEFINITIONS[agentType]
    navigator.clipboard.writeText(def.installCommand)
    setCopiedAgent(agentType)
    setTimeout(() => setCopiedAgent(null), 2000)
  }

  const installedAgents = AGENT_LIST.filter((a) => installStatus[a.type])
  const notInstalledAgents = AGENT_LIST.filter((a) => !installStatus[a.type])

  return (
    <div>
      {/* Header */}
      <div className="flex items-center justify-between mb-1">
        <h2 className="text-xl font-semibold text-white tracking-tight">Coding Agents</h2>
        <button
          onClick={refreshDetection}
          disabled={detectLoading}
          className="flex items-center gap-1.5 px-3 py-1.5 text-xs text-gray-400 hover:text-white
                     bg-white/[0.04] hover:bg-white/[0.08] border border-white/[0.06] hover:border-white/[0.1]
                     rounded-lg transition-all duration-200 disabled:opacity-40 disabled:pointer-events-none"
          title="Re-detect installed agents"
        >
          <RefreshCw
            size={11}
            className={`transition-transform duration-700 ${detectLoading ? 'animate-spin' : ''}`}
          />
          {detectLoading ? 'Scanning...' : 'Re-detect'}
        </button>
      </div>
      <p className="text-sm text-gray-500 mb-6">
        Configure the command and arguments for each coding agent
      </p>

      {/* Status summary bar */}
      <div
        className="flex items-center gap-4 px-4 py-2.5 rounded-lg mb-6 border border-white/[0.04]"
        style={{ background: 'var(--color-surface-sunken)' }}
      >
        <div className="flex items-center gap-2">
          <div className="w-2 h-2 rounded-full bg-emerald-400" />
          <span className="text-xs text-gray-400">
            <span className="text-gray-200 font-medium">{installedAgents.length}</span> installed
          </span>
        </div>
        {notInstalledAgents.length > 0 && (
          <div className="flex items-center gap-2">
            <div className="w-2 h-2 rounded-full bg-gray-600" />
            <span className="text-xs text-gray-400">
              <span className="text-gray-400 font-medium">{notInstalledAgents.length}</span>{' '}
              available
            </span>
          </div>
        )}
        <div className="flex-1" />
        <span className="text-[10px] text-gray-600 uppercase tracking-wider">
          {AGENT_LIST.length} agents
        </span>
      </div>

      {/* Installed Agents */}
      {installedAgents.length > 0 && (
        <div className="mb-8">
          <div className="space-y-2">
            {installedAgents.map((agent) => {
              const def = AGENT_DEFINITIONS[agent.type]
              const cmd = getCommand(agent.type)
              const modified = isModified(agent.type)

              return (
                <div
                  key={agent.type}
                  className="group rounded-xl border border-white/[0.06] hover:border-white/[0.1] transition-all duration-200 overflow-hidden"
                  style={{ background: 'var(--color-surface-sunken)' }}
                >
                  {/* Agent header with brand accent */}
                  <div
                    className="flex items-center gap-3 px-4 py-3"
                    style={{ borderLeft: `2px solid ${def.color}30` }}
                  >
                    <div
                      className="w-9 h-9 rounded-lg flex items-center justify-center shrink-0"
                      style={{ background: def.bgColor }}
                    >
                      <AgentIcon agentType={agent.type} size={18} />
                    </div>
                    <div className="flex-1 min-w-0">
                      <div className="flex items-center gap-2">
                        <span className="text-sm font-medium text-gray-100">
                          {agent.displayName}
                        </span>
                        <span
                          className="inline-flex items-center gap-1 text-[10px] px-1.5 py-0.5 rounded-full font-medium"
                          style={{
                            color: def.color,
                            background: `${def.color}12`
                          }}
                        >
                          <Terminal size={8} />
                          Ready
                        </span>
                      </div>
                      <span className="text-[11px] text-gray-500">{agent.description}</span>
                    </div>
                    {modified && (
                      <button
                        onClick={() => resetAgent(agent.type)}
                        className="flex items-center gap-1 text-[11px] text-gray-500 hover:text-gray-300
                                   px-2 py-1 bg-white/[0.03] hover:bg-white/[0.06]
                                   border border-white/[0.04] hover:border-white/[0.08]
                                   rounded-md transition-all opacity-0 group-hover:opacity-100"
                      >
                        <RotateCcw size={10} />
                        Reset
                      </button>
                    )}
                  </div>

                  {/* Command config */}
                  <div className="px-4 pb-3 pt-1 space-y-2">
                    <div className="grid grid-cols-[140px_1fr] gap-2">
                      <div>
                        <label className="text-[10px] text-gray-600 uppercase tracking-wider mb-1 block">
                          Command
                        </label>
                        <input
                          type="text"
                          value={cmd.command}
                          onChange={(e) =>
                            updateAgentCommand(agent.type, { command: e.target.value })
                          }
                          className="w-full px-2.5 py-1.5 bg-black/30 border border-white/[0.06] rounded-md text-xs
                                     text-gray-200 font-mono focus:border-white/[0.15] focus:outline-none
                                     focus:ring-1 focus:ring-white/[0.06] transition-all"
                        />
                      </div>
                      <div>
                        <label className="text-[10px] text-gray-600 uppercase tracking-wider mb-1 block">
                          Arguments
                        </label>
                        <input
                          type="text"
                          value={getArgsString(agent.type)}
                          onChange={(e) => handleArgsChange(agent.type, e.target.value)}
                          onBlur={() => handleArgsBlur(agent.type)}
                          placeholder="e.g. --dangerously-skip-permissions"
                          className="w-full px-2.5 py-1.5 bg-black/30 border border-white/[0.06] rounded-md text-xs
                                     text-gray-200 font-mono placeholder-gray-700 focus:border-white/[0.15] focus:outline-none
                                     focus:ring-1 focus:ring-white/[0.06] transition-all"
                        />
                      </div>
                    </div>
                    <div>
                      <label className="text-[10px] text-gray-600 uppercase tracking-wider mb-1 block">
                        Headless Arguments
                      </label>
                      <input
                        type="text"
                        value={getHeadlessArgsString(agent.type)}
                        onChange={(e) => handleHeadlessArgsChange(agent.type, e.target.value)}
                        onBlur={() => handleHeadlessArgsBlur(agent.type)}
                        placeholder="e.g. --dangerously-skip-permissions (used for headless/workflow runs)"
                        className="w-full px-2.5 py-1.5 bg-black/30 border border-white/[0.06] rounded-md text-xs
                                   text-gray-200 font-mono placeholder-gray-700 focus:border-white/[0.15] focus:outline-none
                                   focus:ring-1 focus:ring-white/[0.06] transition-all"
                      />
                    </div>
                  </div>
                </div>
              )
            })}
          </div>
        </div>
      )}

      {/* Not Installed Agents */}
      {notInstalledAgents.length > 0 && (
        <div>
          <div className="flex items-center gap-2 mb-3">
            <Download size={12} className="text-gray-600" />
            <h3 className="text-xs font-medium text-gray-500 uppercase tracking-wider">
              Available to Install
            </h3>
          </div>
          <div className="space-y-1.5">
            {notInstalledAgents.map((agent) => {
              const def = AGENT_DEFINITIONS[agent.type]
              const isExpanded = expandedNotInstalled[agent.type]

              return (
                <div
                  key={agent.type}
                  className="rounded-xl border border-white/[0.04] hover:border-white/[0.06] transition-all duration-200 overflow-hidden"
                  style={{ background: 'var(--color-surface-sunken)' }}
                >
                  <button
                    onClick={() =>
                      setExpandedNotInstalled((prev) => ({
                        ...prev,
                        [agent.type]: !prev[agent.type]
                      }))
                    }
                    className="flex items-center gap-3 w-full px-4 py-3 text-left"
                  >
                    <div className="w-9 h-9 rounded-lg flex items-center justify-center shrink-0 bg-white/[0.03] opacity-50">
                      <AgentIcon agentType={agent.type} size={18} />
                    </div>
                    <div className="flex-1 min-w-0">
                      <span className="text-sm font-medium text-gray-400">{agent.displayName}</span>
                      <span className="text-[11px] text-gray-600 ml-2">{agent.description}</span>
                    </div>
                    {isExpanded ? (
                      <ChevronDown size={14} className="text-gray-600 shrink-0" />
                    ) : (
                      <ChevronRight size={14} className="text-gray-600 shrink-0" />
                    )}
                  </button>

                  {/* Install instructions (expandable) */}
                  {isExpanded && (
                    <div className="px-4 pb-4 pt-0">
                      <div className="p-3 bg-black/30 border border-white/[0.06] rounded-lg">
                        <div className="flex items-center gap-2 mb-2">
                          <Terminal size={10} className="text-gray-500" />
                          <span className="text-[10px] text-gray-500 uppercase tracking-wider font-medium">
                            Install command
                          </span>
                        </div>
                        <div className="flex items-center gap-2">
                          <div className="flex-1 flex items-center gap-2 px-3 py-2 bg-white/[0.03] border border-white/[0.04] rounded-md">
                            <span className="text-gray-600 text-xs select-none">$</span>
                            <code className="text-xs font-mono text-gray-300 select-all flex-1">
                              {def.installCommand}
                            </code>
                          </div>
                          <button
                            onClick={(e) => {
                              e.stopPropagation()
                              copyInstallCommand(agent.type)
                            }}
                            className="p-2 text-gray-500 hover:text-white bg-white/[0.04]
                                       hover:bg-white/[0.08] border border-white/[0.04] hover:border-white/[0.08]
                                       rounded-lg transition-all duration-200 shrink-0"
                            title="Copy install command"
                          >
                            {copiedAgent === agent.type ? (
                              <CheckCircle size={14} className="text-emerald-400" />
                            ) : (
                              <Copy size={14} />
                            )}
                          </button>
                          <a
                            href={def.installUrl}
                            target="_blank"
                            rel="noopener noreferrer"
                            className="p-2 text-gray-500 hover:text-white bg-white/[0.04]
                                       hover:bg-white/[0.08] border border-white/[0.04] hover:border-white/[0.08]
                                       rounded-lg transition-all duration-200 shrink-0"
                            title="View documentation"
                            onClick={(e) => e.stopPropagation()}
                          >
                            <ExternalLink size={14} />
                          </a>
                        </div>
                        <p className="text-[10px] text-gray-600 mt-2">
                          After installing, click Re-detect above to enable this agent.
                        </p>
                      </div>
                    </div>
                  )}
                </div>
              )
            })}
          </div>
        </div>
      )}
    </div>
  )
}
