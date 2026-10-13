import type { ConnectorListing } from '../../../lib/connector-browse'
import {
  requirementAction,
  type RequirementAction,
  type TemplateRequirement
} from '../../../lib/template-requirements'
import { describeRequirement } from '../../../lib/workflow-files'

/** Each unmet requirement offers the one step that answers it. */
const ACTION_LABEL: Partial<Record<RequirementAction['kind'], (a: RequirementAction) => string>> = {
  install: (a) => (a.kind === 'install' ? `Install ${a.listing.name}` : 'Install'),
  addConnection: () => 'Add connection',
  createProfile: () => 'Create profile'
}

/**
 * What one unmet requirement says, and what can be done about it here.
 *
 * Shared by the template list and by a workflow an import left unbound: the
 * same sentence with the same button, because it is the same problem.
 */
export function RequirementRow({
  requirement,
  listings,
  onFix
}: {
  requirement: TemplateRequirement
  listings: ConnectorListing[]
  onFix?: (action: RequirementAction) => void
}) {
  const action = requirementAction(requirement, listings)
  const label = ACTION_LABEL[action.kind]?.(action)

  return (
    <div className="pl-[38px] pr-2.5 pb-1.5 flex items-center gap-2">
      <span className="min-w-0 flex-1 text-[11px] text-bronzo truncate">
        Needs {describeRequirement(requirement.requirement)}
      </span>
      {label && onFix && (
        <button
          onClick={() => onFix(action)}
          className="shrink-0 text-[11px] text-gray-300 hover:text-white px-2 py-0.5 border
                     border-white/[0.1] rounded-sm hover:bg-white/[0.06] transition-colors"
        >
          {label}
        </button>
      )}
    </div>
  )
}
