import { DEFAULT_GATE_ROUNDS, MAX_GATE_ROUNDS } from '@vornrun/shared/workflow-graph'
import type { StepVariableGroup, TemplateVariable } from '@vornrun/shared/template-vars'
import type { ApprovalConfig, GateFeedbackConfig, WorkflowNode } from '../../../../shared/types'
import { ToggleSwitch } from '../../settings/ToggleSwitch'
import { VariableAutocomplete } from './VariableAutocomplete'

interface Props {
  config: ApprovalConfig
  onChange: (config: ApprovalConfig) => void
  stepGroups?: StepVariableGroup[]
  inputVars?: TemplateVariable[]
  /** Steps above this gate, which it can send the work back to. */
  redoFromSteps?: WorkflowNode[]
  /** This gate's slug, for the variable its comment is read through. */
  slug?: string
}

const LABEL = 'text-[13px] text-gray-400 font-medium block mb-2'
const HELP = 'mt-1.5 text-[11px] leading-[1.45] text-gray-500'
const FIELD =
  'px-3 py-2 bg-white/[0.03] border border-white/[0.08] rounded-md text-[13px] text-gray-200 placeholder:text-gray-600 focus:outline-none focus:border-white/[0.2]'

export function ApprovalConfigForm({
  config,
  onChange,
  stepGroups = [],
  inputVars = [],
  redoFromSteps = [],
  slug
}: Props) {
  const timeoutSeconds =
    config.timeoutMs && config.timeoutMs > 0 ? Math.round(config.timeoutMs / 1000) : ''
  const steps = redoFromSteps.filter((n) => n.type !== 'trigger')
  const feedback = config.feedback

  const setFeedback = (next: Partial<GateFeedbackConfig>): void =>
    onChange({
      ...config,
      feedback: {
        from: feedback?.from ?? steps[0]?.id ?? '',
        maxRounds: feedback?.maxRounds ?? DEFAULT_GATE_ROUNDS,
        ...next
      }
    })
  const clearFeedback = (): void => {
    const { feedback: _off, ...rest } = config
    onChange(rest)
  }

  return (
    <div className="space-y-5">
      <div>
        <label className={LABEL}>Message</label>
        <VariableAutocomplete
          value={config.message || ''}
          onChange={(message) => onChange({ ...config, message })}
          placeholder="Shown with the answers, e.g. {{steps.draft.output}}"
          rows={3}
          stepGroups={stepGroups}
          contextVars={inputVars}
        />
        <div className={HELP}>Step outputs fill in when the gate opens.</div>
      </div>

      <div>
        <label className={LABEL}>
          Editable text <span className="text-gray-600">(optional)</span>
        </label>
        <VariableAutocomplete
          value={config.edit || ''}
          onChange={(edit) => onChange({ ...config, edit: edit || undefined })}
          placeholder="{{steps.draft.output}}"
          rows={1}
          stepGroups={stepGroups}
          contextVars={inputVars}
          mono
        />
        <div className={HELP}>
          The reviewer may rewrite this before approving. Later steps read{' '}
          <code className="font-mono text-gray-400">{`{{steps.${slug || 'this_gate'}.text}}`}</code>
          , which is the rewrite when there is one and the original otherwise.
        </div>
      </div>

      <div>
        <label className={LABEL}>
          Review page <span className="text-gray-600">(optional)</span>
        </label>
        <VariableAutocomplete
          value={config.view || ''}
          onChange={(view) => onChange({ ...config, view: view || undefined })}
          placeholder="{{steps.review_page.output}}"
          rows={1}
          stepGroups={stepGroups}
          contextVars={inputVars}
          mono
        />
        <div className={HELP}>
          HTML, or the path of a .html file. Opens beside the answers, sandboxed, with no network.
        </div>
      </div>

      <div>
        <div className="flex items-center justify-between gap-3">
          <span className="text-[13px] text-gray-400 font-medium">
            Reviewer can request changes
          </span>
          <ToggleSwitch
            checked={!!feedback}
            disabled={!feedback && steps.length === 0}
            onChange={(on) => (on ? setFeedback({}) : clearFeedback())}
          />
        </div>
        {!feedback && steps.length === 0 && (
          <div className={HELP}>Add a step before this gate to send the work back to.</div>
        )}
        {feedback && (
          <div className="mt-3 space-y-2.5">
            <div className="flex items-center gap-2 text-[12.5px] text-gray-400">
              <label htmlFor="approval-feedback-from" className="shrink-0">
                Redo from
              </label>
              <select
                id="approval-feedback-from"
                value={feedback.from}
                onChange={(e) => setFeedback({ from: e.target.value })}
                className={`${FIELD} flex-1 min-w-0 py-1.5`}
              >
                {!steps.some((s) => s.id === feedback.from) && (
                  <option value={feedback.from}>Pick a step</option>
                )}
                {steps.map((s) => (
                  <option key={s.id} value={s.id}>
                    {s.label || s.id}
                  </option>
                ))}
              </select>
            </div>
            <div className="flex items-center gap-2 text-[12.5px] text-gray-400">
              <label htmlFor="approval-feedback-rounds">up to</label>
              <input
                id="approval-feedback-rounds"
                type="number"
                min={2}
                max={MAX_GATE_ROUNDS}
                value={feedback.maxRounds}
                onChange={(e) => {
                  const rounds = Math.floor(Number(e.target.value))
                  if (Number.isFinite(rounds)) {
                    setFeedback({ maxRounds: Math.min(Math.max(rounds, 2), MAX_GATE_ROUNDS) })
                  }
                }}
                className={`${FIELD} w-16 py-1.5 text-center font-mono text-[12px]`}
              />
              <span>rounds</span>
            </div>
            <div className={HELP}>
              Those steps run again with the comment, read as{' '}
              <code className="font-mono text-gray-400">{`{{steps.${slug || 'this_gate'}.feedback}}`}</code>
              , then the gate asks again.
            </div>
          </div>
        )}
      </div>

      <div>
        <label className={LABEL}>
          Timeout <span className="text-gray-600">(seconds, optional)</span>
        </label>
        <input
          type="number"
          min={0}
          value={timeoutSeconds}
          onChange={(e) => {
            const raw = e.target.value.trim()
            const secs = Number(raw)
            const valid = raw !== '' && Number.isFinite(secs) && secs > 0
            onChange({ ...config, timeoutMs: valid ? Math.round(secs * 1000) : undefined })
          }}
          placeholder="Leave blank to wait forever"
          className={`${FIELD} w-full`}
        />
        <div className={HELP}>
          Auto-rejects the gate if nobody answers in time. Each round starts a new timeout.
        </div>
      </div>
    </div>
  )
}
