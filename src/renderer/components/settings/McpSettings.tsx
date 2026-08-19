import { useState } from 'react'
import { AGENT_DEFINITIONS } from '../../lib/agent-definitions'
import { AGENT_MCP_SETUPS } from '../../lib/mcp-data'
import { AgentIcon } from '../AgentIcon'

function CopyButton({ text }: { text: string }) {
  const [copied, setCopied] = useState(false)

  const handleCopy = (): void => {
    navigator.clipboard.writeText(text)
    setCopied(true)
    setTimeout(() => setCopied(false), 2000)
  }

  return (
    <button
      onClick={handleCopy}
      className="text-xs px-2 py-1 rounded bg-white/[0.04] hover:bg-white/[0.08]
                 text-gray-400 hover:text-gray-200 transition-colors shrink-0"
    >
      {copied ? 'Copied' : 'Copy'}
    </button>
  )
}

export function McpSettings() {
  return (
    <div>
      <h2 className="text-xl font-semibold text-white mb-1">Skills &amp; MCP</h2>
      <p className="text-sm text-gray-500 mb-6">
        Connect your coding agents to Vorn. Each setup below installs the MCP server along with the
        guidance that explains what its tools are for.
      </p>

      {/* Per-agent commands */}
      <h3 className="text-sm font-medium text-gray-200 mb-3">Set up an agent</h3>
      <div className="space-y-3">
        {AGENT_MCP_SETUPS.map((setup) => {
          const agent = AGENT_DEFINITIONS[setup.agentType]

          return (
            <div
              key={setup.agentType}
              className="border border-white/[0.06] rounded-lg p-4"
              style={{ background: 'var(--color-surface-sunken)' }}
            >
              <div className="flex items-center gap-3 mb-3">
                <AgentIcon agentType={setup.agentType} size={20} />
                <span className="text-sm font-medium text-gray-200">{agent.displayName}</span>
                <span className="text-[11px] text-gray-500">
                  {setup.inAgent ? `Run in ${agent.displayName}` : 'Run in your terminal'}
                </span>
              </div>
              <div className="space-y-2">
                {setup.commands.map((command) => (
                  <div key={command} className="flex items-center gap-2">
                    <code
                      className="flex-1 px-3 py-1.5 bg-white/[0.04] border border-white/[0.08] rounded-md
                               text-xs text-gray-300 font-mono overflow-x-auto whitespace-nowrap"
                    >
                      {command}
                    </code>
                    <CopyButton text={command} />
                  </div>
                ))}
              </div>
              {setup.note && <p className="mt-2 text-[11px] text-gray-500">{setup.note}</p>}
            </div>
          )
        })}
      </div>
    </div>
  )
}
