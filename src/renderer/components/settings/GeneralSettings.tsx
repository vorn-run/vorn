import { useAppStore } from '../../stores'
import { ShellPicker } from './ShellPicker'
import { AGENT_LIST } from '../../lib/agent-definitions'
import { AgentIcon } from '../AgentIcon'
import { AiAgentType } from '../../../shared/types'
import { useAgentInstallStatus } from '../../hooks/useAgentInstallStatus'
import { isElectron } from '../../lib/platform'
import { SettingsPageHeader } from './SettingsPageHeader'
import { SettingRow } from './SettingRow'
import { ToggleSwitch } from './ToggleSwitch'
import { DEFAULT_STEP_TIMEOUT_MINUTES } from '../../lib/workflow-execution'

export function GeneralSettings() {
  const config = useAppStore((s) => s.config)
  const setConfig = useAppStore((s) => s.setConfig)
  const { status: installStatus } = useAgentInstallStatus()

  if (!config) return null

  // Two ordinary settings quietly override the sentence above, and a promise
  // that is conditional should say so where it is made rather than leaving the
  // user to notice a server that never leaves.
  // Read the way `scheduler.getTriggerConfig` reads it -- first trigger node,
  // its own config -- so this cannot describe a different set of workflows from
  // the one the server actually stays awake for.
  const polling = (config.workflows ?? []).some((w) => {
    if (!w.enabled) return false
    const trigger = w.nodes.find((n) => n.type === 'trigger')
    return (
      (trigger?.config as { triggerType?: string } | undefined)?.triggerType === 'connectorPoll'
    )
  })
  const reasons = [
    config.defaults.networkAccessEnabled
      ? 'Network Access is on, so it stays up to be reached from your other devices'
      : null,
    polling ? 'a connector is polling on a schedule, which it services itself' : null
  ].filter(Boolean)
  const serverStaysUp =
    reasons.length === 0
      ? undefined
      : `${reasons.join(', and ')}. This server will not shut down on its own.`

  const updateDefaults = (patch: Partial<typeof config.defaults>): void => {
    const updated = {
      ...config,
      defaults: { ...config.defaults, ...patch }
    }
    window.api.saveConfig(updated)
    setConfig(updated)
  }

  return (
    <div>
      <SettingsPageHeader title="General" description="Application preferences and defaults" />

      <div className="space-y-1">
        {/* Default Coding Agent */}
        <SettingRow
          label="Default Coding Agent"
          description="Pre-selected agent when creating new sessions"
        >
          <div className="flex bg-white/[0.04] rounded-lg p-0.5 gap-0.5">
            {AGENT_LIST.map((agent) => {
              const installed = installStatus[agent.type]
              return (
                <button
                  key={agent.type}
                  onClick={() =>
                    installed && updateDefaults({ defaultAgent: agent.type as AiAgentType })
                  }
                  disabled={!installed}
                  className={`flex items-center gap-1.5 px-2.5 py-1 rounded-md text-xs transition-colors ${
                    !installed
                      ? 'opacity-30 cursor-not-allowed text-gray-600'
                      : (config.defaults.defaultAgent || 'claude') === agent.type
                        ? 'bg-white/[0.1] text-white'
                        : 'text-gray-500 hover:text-gray-300'
                  }`}
                  title={!installed ? `${agent.displayName} is not installed` : agent.displayName}
                >
                  <AgentIcon agentType={agent.type} size={14} />
                  <span className="hidden sm:inline">{agent.displayName.split(' ')[0]}</span>
                </button>
              )
            })}
          </div>
        </SettingRow>

        {/* Default Shell */}
        <SettingRow
          label="Default Shell"
          description="Shell used for terminal sessions. Shells differ in how much they can report about each command."
        >
          <ShellPicker
            value={config.defaults.shell}
            onChange={(shell) => updateDefaults({ shell })}
          />
        </SettingRow>

        {/* Start at login: above Reopen Sessions because it changes what that one does */}
        {isElectron && (
          <SettingRow
            label="Start Vorn When I Sign In"
            description="Vorn opens with the machine, so a restart puts you back where you were."
            note={
              config.defaults.reopenSessions !== false
                ? 'With Reopen Sessions on, your agents start again at sign-in rather than when you next open Vorn.'
                : undefined
            }
          >
            <ToggleSwitch
              checked={config.defaults.startAtLogin === true}
              onChange={(startAtLogin) => updateDefaults({ startAtLogin })}
            />
          </SettingRow>
        )}

        {/* Reopen Sessions */}
        <SettingRow
          label="Reopen Sessions on Startup"
          description="Bring back the panes you had open. Agents still running are reconnected to, keeping the turn they were in; ones whose process was stopped are started again where they left off, and shells reopen in the directory they were in."
        >
          <ToggleSwitch
            checked={config.defaults.reopenSessions ?? true}
            onChange={(reopenSessions) => updateDefaults({ reopenSessions })}
          />
        </SettingRow>

        {/* Keep sessions running */}
        <SettingRow
          label="Keep Sessions Running When Vorn Closes"
          description="Agents keep working while Vorn is shut. Reopening reconnects to them instead of restarting them. A background server stays running until every session ends, then shuts down on its own."
          note={serverStaysUp}
        >
          <ToggleSwitch
            checked={config.defaults.keepSessionsRunning ?? true}
            onChange={(keepSessionsRunning) => updateDefaults({ keepSessionsRunning })}
          />
        </SettingRow>

        {/* Block rendering */}
        <SettingRow
          label="Command Blocks"
          description="Draw finished commands as blocks with their own padding, boundary and copy button, instead of leaving them in the terminal grid"
        >
          <ToggleSwitch
            checked={config.defaults.domBlockRendering ?? true}
            onChange={(domBlockRendering) => updateDefaults({ domBlockRendering })}
          />
        </SettingRow>

        {/* Minimal shell prompt */}
        <SettingRow
          label="Minimal Shell Prompt"
          description="Replace your shell prompt in terminal sessions so each command reads as a heading. Turn off to keep your own prompt exactly as your shell renders it"
        >
          <ToggleSwitch
            checked={config.defaults.minimalShellPrompt ?? true}
            onChange={(minimalShellPrompt) => updateDefaults({ minimalShellPrompt })}
          />
        </SettingRow>

        {/* Hover Preview */}
        <SettingRow
          label="Hover Preview"
          description="Preview sessions by hovering over them in the sidebar"
        >
          <ToggleSwitch
            checked={config.defaults.enableHoverPreview ?? false}
            onChange={(enableHoverPreview) => updateDefaults({ enableHoverPreview })}
          />
        </SettingRow>

        {/* Floating Widget — Electron only */}
        {isElectron && (
          <SettingRow
            label="Floating Widget"
            description="Show agent status widget when app is not focused"
          >
            <ToggleSwitch
              checked={config.defaults.widgetEnabled === true}
              onChange={(enabled) => {
                updateDefaults({ widgetEnabled: enabled })
                window.api.setWidgetEnabled(enabled)
              }}
            />
          </SettingRow>
        )}

        {/* Show Headless Agents */}
        <SettingRow
          label="Show Headless Agents"
          description="Display background agent sessions above the session grid"
        >
          <ToggleSwitch
            checked={config.defaults.showHeadlessAgents !== false}
            onChange={(showHeadlessAgents) => updateDefaults({ showHeadlessAgents })}
          />
        </SettingRow>

        {/* Completed Agent Retention */}
        {config.defaults.showHeadlessAgents !== false && (
          <SettingRow
            label="Completed Agent Retention"
            description="How long to show completed headless agents before auto-hiding"
          >
            <select
              value={config.defaults.headlessRetentionMinutes ?? 1}
              onChange={(e) => updateDefaults({ headlessRetentionMinutes: +e.target.value })}
              className="w-32 px-3 py-1.5 bg-white/[0.04] border border-white/[0.08] rounded-md text-sm
                         text-gray-200 focus:border-white/[0.15] focus:outline-none"
            >
              <option value={1}>1 minute</option>
              <option value={5}>5 minutes</option>
              <option value={15}>15 minutes</option>
              <option value={30}>30 minutes</option>
              <option value={60}>1 hour</option>
            </select>
          </SettingRow>
        )}

        {/* Workflow Step Timeout */}
        <SettingRow
          label="Workflow Step Timeout"
          description="How long a headless workflow step may run before its agent is killed and the step fails. Individual steps can override this."
        >
          <select
            value={config.defaults.headlessStepTimeoutMinutes ?? DEFAULT_STEP_TIMEOUT_MINUTES}
            onChange={(e) => updateDefaults({ headlessStepTimeoutMinutes: +e.target.value })}
            className="w-32 px-3 py-1.5 bg-white/[0.04] border border-white/[0.08] rounded-md text-sm
                       text-gray-200 focus:border-white/[0.15] focus:outline-none"
          >
            <option value={15}>15 minutes</option>
            <option value={30}>30 minutes</option>
            <option value={60}>1 hour</option>
            <option value={120}>2 hours</option>
            <option value={360}>6 hours</option>
            <option value={0}>No limit</option>
          </select>
        </SettingRow>
      </div>
    </div>
  )
}
