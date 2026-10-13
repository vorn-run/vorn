import { X, FilePlus2, Check, Plug } from 'lucide-react'
import type { SourceConnection, WorkflowTemplate } from '../../../../shared/types'
import {
  templateRequirements,
  type ConnectorSuggestion,
  type RequirementAction
} from '../../../lib/template-requirements'
import type { ConnectorListing } from '../../../lib/connector-browse'
import { RequirementRow } from './RequirementRow'

/**
 * What a new workflow can start from, before the canvas is anything.
 *
 * Blank comes first because a template is an offer, not a gate. Each row says
 * what its steps are and what it still wants connected, so the choice is made
 * before the canvas fills rather than after.
 */
export function StartFromPanel({
  templates,
  connections,
  listings = [],
  suggestions = [],
  onPickBlank,
  onPickTemplate,
  onPickSuggestion,
  onFixRequirement,
  onClose,
  children
}: {
  templates: WorkflowTemplate[]
  connections: SourceConnection[]
  /** Every connector this machine could reach, so a requirement can offer its own fix. */
  listings?: ConnectorListing[]
  suggestions?: ConnectorSuggestion[]
  onPickBlank: () => void
  onPickTemplate: (template: WorkflowTemplate) => void
  onPickSuggestion?: (suggestion: ConnectorSuggestion) => void
  onFixRequirement?: (action: RequirementAction) => void
  onClose: () => void
  /** The sheet or form a requirement's action opened, shown over the list. */
  children?: React.ReactNode
}) {
  return (
    <div
      data-start-from
      className="w-[280px] border-l border-white/[0.08] bg-surface-node flex flex-col h-full overflow-hidden titlebar-no-drag"
      onKeyDown={(e) => {
        if (e.key !== 'Escape') return
        e.stopPropagation()
        onClose()
      }}
    >
      <div className="px-4 py-3 border-b border-white/[0.08] flex items-center justify-between">
        <span className="text-[13px] font-medium text-white">Start from</span>
        <button
          aria-label="Close"
          onClick={onClose}
          className="p-1 rounded-md text-gray-500 hover:text-white hover:bg-white/[0.06] transition-colors"
        >
          <X size={14} />
        </button>
      </div>

      {children}

      <div className="flex-1 overflow-y-auto p-2">
        <button
          onClick={onPickBlank}
          className="w-full text-left px-2.5 py-2 rounded-md flex items-start gap-2.5
                     text-gray-300 hover:text-white hover:bg-white/[0.06] transition-colors"
        >
          <FilePlus2 size={14} strokeWidth={1.5} className="shrink-0 mt-0.5 text-gray-500" />
          <span className="min-w-0">
            <span className="block text-[12px] font-medium">Blank canvas</span>
            <span className="block text-[11px] text-gray-500">Pick your own trigger</span>
          </span>
        </button>

        {templates.length > 0 && (
          <div className="px-2 pt-3 pb-1 text-[10px] font-mono uppercase tracking-wider text-gray-600">
            Templates
          </div>
        )}

        {templates.map((template) => (
          <TemplateRow
            key={template.id}
            template={template}
            connections={connections}
            listings={listings}
            onPick={() => onPickTemplate(template)}
            onFix={onFixRequirement}
          />
        ))}

        {suggestions.length > 0 && (
          <div className="px-2 pt-3 pb-1 text-[10px] font-mono uppercase tracking-wider text-gray-600">
            From your connections
          </div>
        )}

        {suggestions.map((suggestion) => (
          <button
            key={suggestion.key}
            onClick={() => onPickSuggestion?.(suggestion)}
            className="w-full text-left px-2.5 py-2 rounded-md flex items-start gap-2.5
                       text-gray-300 hover:text-white hover:bg-white/[0.06] transition-colors"
          >
            <Plug size={14} strokeWidth={1.5} className="shrink-0 mt-0.5 text-gray-500" />
            <span className="min-w-0 flex-1">
              <span className="block text-[12px] font-medium truncate">{suggestion.name}</span>
              <span className="block text-[11px] text-gray-500 truncate">
                {suggestion.connectionName}
              </span>
            </span>
          </button>
        ))}
      </div>
    </div>
  )
}

function TemplateRow({
  template,
  connections,
  listings,
  onPick,
  onFix
}: {
  template: WorkflowTemplate
  connections: SourceConnection[]
  listings: ConnectorListing[]
  onPick: () => void
  onFix?: (action: RequirementAction) => void
}) {
  const requirements = templateRequirements(template, connections)
  const pending = requirements.filter((entry) => entry.connectionId === undefined)

  return (
    // A row, not a button: what a requirement wants doing is its own control,
    // and one cannot sit inside the other.
    <div className="rounded-md hover:bg-white/[0.06] transition-colors">
      <button
        onClick={onPick}
        className="w-full text-left px-2.5 py-2 flex items-start gap-2.5 text-gray-300 hover:text-white"
      >
        <StepStrip count={template.portable.nodes.length} />
        <span className="min-w-0 flex-1">
          <span className="block text-[12px] font-medium truncate">{template.name}</span>
          <span className="block text-[11px] text-gray-500 truncate">
            {template.steps.join(' · ')}
          </span>
          {pending.length === 0 && requirements.length > 0 && (
            <span className="flex items-center gap-1 text-[11px] text-status-sage">
              <Check size={10} strokeWidth={2} />
              Connected
            </span>
          )}
        </span>
      </button>

      {pending.map((entry) => (
        <RequirementRow
          key={entry.requirement.nodeId + entry.requirement.kind}
          requirement={entry}
          listings={listings}
          onFix={onFix}
        />
      ))}
    </div>
  )
}

/** The shape of the flow, drawn rather than counted. */
function StepStrip({ count }: { count: number }) {
  return (
    <span
      aria-hidden
      className="shrink-0 mt-0.5 w-[22px] flex flex-col items-center gap-[3px] opacity-70"
    >
      {Array.from({ length: Math.min(count, 4) }).map((_, i) => (
        <span key={i} className="w-full h-[3px] rounded-[1px] bg-white/[0.18]" />
      ))}
    </span>
  )
}
