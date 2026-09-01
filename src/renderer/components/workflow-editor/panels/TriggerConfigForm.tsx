import {
  Zap,
  Clock,
  RefreshCw,
  ListPlus,
  ArrowRightLeft,
  Plug,
  Globe,
  Copy,
  Check
} from 'lucide-react'
import { useEffect, useState } from 'react'
import { useAppStore } from '../../../stores'
import { TriggerConfig, TaskStatus } from '../../../../shared/types'
import { SelectPicker } from '../../SelectPicker'
import { ProjectPicker } from '../../ProjectPicker'
import { ConnectorPollTriggerForm } from './ConnectorPollTriggerForm'
import { WorkflowInputsEditor } from './WorkflowInputsEditor'

interface Props {
  config: TriggerConfig
  onChange: (config: TriggerConfig) => void
  /** Open the step library in trigger scope to swap this trigger. */
  onOpenLibrary?: () => void
}

const CRON_PRESETS = [
  { label: 'Weekdays 9am', value: '0 9 * * 1-5' },
  { label: 'Daily 9am', value: '0 9 * * *' },
  { label: 'Every hour', value: '0 * * * *' },
  { label: 'Every 30min', value: '*/30 * * * *' },
  { label: 'Weekly (Mon 9am)', value: '0 9 * * 1' }
]

const TRIGGER_TYPES = [
  {
    type: 'manual' as const,
    label: 'Manual',
    icon: Zap,
    hint: 'Run manually from the play button'
  },
  { type: 'once' as const, label: 'Once', icon: Clock, hint: 'Runs once at the scheduled time' },
  {
    type: 'recurring' as const,
    label: 'Recurring',
    icon: RefreshCw,
    hint: 'Runs on a repeating schedule'
  },
  {
    type: 'taskCreated' as const,
    label: 'Task Created',
    icon: ListPlus,
    hint: 'Fires when a new task is added'
  },
  {
    type: 'taskStatusChanged' as const,
    label: 'Status Change',
    icon: ArrowRightLeft,
    hint: "Fires when a task's status changes"
  },
  {
    type: 'connectorPoll' as const,
    label: 'Connector Poll',
    icon: Plug,
    hint: 'Polls an external connector on cron and fires per new item'
  },
  {
    type: 'webhook' as const,
    label: 'Webhook',
    icon: Globe,
    hint: 'Fires when this machine receives an HTTP request at the workflow URL'
  }
]

const STATUS_PICKER_OPTIONS = [
  { value: '', label: 'Any status' },
  { value: 'todo', label: 'To Do' },
  { value: 'in_progress', label: 'In Progress' },
  { value: 'in_review', label: 'In Review' },
  { value: 'done', label: 'Done' },
  { value: 'cancelled', label: 'Cancelled' }
]

const EMPTY_PROJECTS: import('../../../../shared/types').ProjectConfig[] = []

export function TriggerConfigForm({ config, onChange, onOpenLibrary }: Props) {
  const projects = useAppStore((s) => s.config?.projects ?? EMPTY_PROJECTS)

  return (
    <div className="space-y-5">
      <div>
        <label className="text-[13px] text-gray-400 font-medium block mb-2">Trigger Type</label>
        {(() => {
          const current = TRIGGER_TYPES.find((t) => t.type === config.triggerType)
          const Icon = current?.icon
          return (
            <div className="flex items-center gap-2 px-3 py-2 rounded-lg border border-white/[0.06] bg-white/[0.02]">
              {Icon && <Icon size={13} className="text-gray-400 shrink-0" />}
              <span className="text-[13px] text-gray-200">
                {current?.label ?? config.triggerType}
              </span>
            </div>
          )
        })()}
        <p className="text-[11px] text-gray-500 mt-1.5">
          {TRIGGER_TYPES.find((t) => t.type === config.triggerType)?.hint}
        </p>
        {onOpenLibrary && (
          <button
            onClick={onOpenLibrary}
            className="mt-2 text-[11px] text-gray-500 hover:text-gray-300 underline underline-offset-2 transition-colors"
          >
            Change trigger from the library
          </button>
        )}
      </div>

      {config.triggerType === 'manual' &&
        (() => {
          const isContextual = config.contextual === true
          return (
            <>
              <button
                role="switch"
                aria-checked={isContextual}
                onClick={() =>
                  onChange({
                    ...config,
                    contextual: isContextual ? undefined : true
                  })
                }
                className={`flex items-center gap-3 w-full px-3 py-2.5 rounded-lg border transition-all ${
                  isContextual
                    ? 'border-white/[0.1] bg-white/[0.04]'
                    : 'border-white/[0.04] bg-white/[0.02] hover:border-white/[0.1]'
                }`}
              >
                <div
                  className={`w-7 h-[16px] rounded-full transition-colors relative shrink-0 ${
                    isContextual ? 'bg-gray-400' : 'bg-white/[0.1]'
                  }`}
                >
                  <div
                    className={`absolute top-[2px] w-[12px] h-[12px] rounded-full bg-white transition-transform ${
                      isContextual ? 'translate-x-[13px]' : 'translate-x-[2px]'
                    }`}
                  />
                </div>
                <div className="text-left min-w-0">
                  <div className="flex items-center gap-1.5">
                    <Zap size={12} className={isContextual ? 'text-gray-300' : 'text-gray-500'} />
                    <span
                      className={`text-[12px] ${isContextual ? 'text-gray-200' : 'text-gray-400'}`}
                    >
                      Contextual
                    </span>
                  </div>
                  <p className="text-[11px] text-gray-500 mt-0.5">
                    Run this workflow directly from any card or terminal, against that session's
                    folder and branch.
                  </p>
                </div>
              </button>
              <WorkflowInputsEditor
                inputs={config.inputs ?? []}
                onChange={(inputs) =>
                  onChange({ ...config, inputs: inputs.length > 0 ? inputs : undefined })
                }
              />
            </>
          )
        })()}

      {config.triggerType === 'once' && (
        <div>
          <label className="text-[13px] text-gray-400 font-medium block mb-2">Run At</label>
          <input
            type="datetime-local"
            value={config.runAt ? new Date(config.runAt).toISOString().slice(0, 16) : ''}
            onChange={(e) =>
              onChange({ triggerType: 'once', runAt: new Date(e.target.value).toISOString() })
            }
            className="w-full px-3 py-2 text-[13px] bg-white/[0.06] border border-white/[0.1] rounded-md
                       text-white focus:outline-none focus:border-white/[0.2] [color-scheme:dark]"
          />
        </div>
      )}

      {config.triggerType === 'recurring' && (
        <>
          <div>
            <label className="text-[13px] text-gray-400 font-medium block mb-2">Preset</label>
            <SelectPicker
              value={config.cron}
              options={CRON_PRESETS.map((p) => ({ value: p.value, label: p.label }))}
              onChange={(v) => onChange({ ...config, cron: v })}
              variant="form"
              placeholder="Choose preset..."
            />
          </div>
          <div>
            <label className="text-[13px] text-gray-400 font-medium block mb-2">
              Cron Expression
            </label>
            <input
              type="text"
              value={config.cron}
              onChange={(e) => onChange({ ...config, cron: e.target.value })}
              placeholder="* * * * *"
              className="w-full px-3 py-2 text-[13px] bg-white/[0.06] border border-white/[0.1] rounded-md
                         text-white placeholder:text-gray-600 focus:outline-none focus:border-white/[0.2] font-mono"
            />
            <p className="text-[11px] text-gray-500 mt-1">min hour day month weekday</p>
          </div>
          <div>
            <label className="text-[13px] text-gray-400 font-medium block mb-2">Timezone</label>
            <input
              type="text"
              value={config.timezone || Intl.DateTimeFormat().resolvedOptions().timeZone}
              onChange={(e) => onChange({ ...config, timezone: e.target.value })}
              className="w-full px-3 py-2 text-[13px] bg-white/[0.06] border border-white/[0.1] rounded-md
                         text-white focus:outline-none focus:border-white/[0.2]"
            />
          </div>
        </>
      )}

      {config.triggerType === 'taskCreated' && (
        <div>
          <label className="text-[13px] text-gray-400 font-medium block mb-2">Project Filter</label>
          <ProjectPicker
            currentProject={config.projectFilter || ''}
            projects={projects}
            onChange={(name) => onChange({ ...config, projectFilter: name || undefined })}
            variant="form"
            allowNone
          />
          <p className="text-[11px] text-gray-500 mt-1">Only trigger for tasks in this project</p>
        </div>
      )}

      {config.triggerType === 'connectorPoll' && (
        <ConnectorPollTriggerForm config={config} onChange={onChange} />
      )}

      {config.triggerType === 'webhook' && (
        <WebhookTriggerFields config={config} onChange={onChange} />
      )}

      {config.triggerType === 'taskStatusChanged' && (
        <>
          <div>
            <label className="text-[13px] text-gray-400 font-medium block mb-2">
              Project Filter
            </label>
            <ProjectPicker
              currentProject={config.projectFilter || ''}
              projects={projects}
              onChange={(name) => onChange({ ...config, projectFilter: name || undefined })}
              variant="form"
              allowNone
            />
          </div>
          <div>
            <label className="text-[13px] text-gray-400 font-medium block mb-2">From Status</label>
            <SelectPicker
              value={config.fromStatus || ''}
              options={STATUS_PICKER_OPTIONS}
              onChange={(v) =>
                onChange({ ...config, fromStatus: (v || undefined) as TaskStatus | undefined })
              }
              placeholder="Any status"
              variant="form"
            />
          </div>
          <div>
            <label className="text-[13px] text-gray-400 font-medium block mb-2">To Status</label>
            <SelectPicker
              value={config.toStatus || ''}
              options={STATUS_PICKER_OPTIONS}
              onChange={(v) =>
                onChange({ ...config, toStatus: (v || undefined) as TaskStatus | undefined })
              }
              placeholder="Any status"
              variant="form"
            />
          </div>
        </>
      )}
    </div>
  )
}

function WebhookTriggerFields({
  config,
  onChange
}: {
  config: Extract<TriggerConfig, { triggerType: 'webhook' }>
  onChange: (config: TriggerConfig) => void
}) {
  const [baseUrl, setBaseUrl] = useState<string | null>(null)
  const [copied, setCopied] = useState(false)

  useEffect(() => {
    let cancelled = false
    window.api
      .getWebhookInfo()
      .then((info) => {
        if (!cancelled) setBaseUrl(info.baseUrl)
      })
      .catch(() => {})
    return () => {
      cancelled = true
    }
  }, [])

  const workflowId = useAppStore((s) => s.editingWorkflowId)
  const url = baseUrl && workflowId ? `${baseUrl}/wf-hooks/${workflowId}/${config.token}` : null

  return (
    <>
      <div>
        <label className="text-[13px] text-gray-400 font-medium block mb-2">URL</label>
        {url ? (
          <div className="flex items-center gap-2">
            <code className="flex-1 min-w-0 px-3 py-2 text-[11px] bg-white/[0.06] border border-white/[0.1] rounded-md text-gray-300 font-mono truncate">
              {url}
            </code>
            <button
              aria-label="Copy URL"
              onClick={() => {
                void navigator.clipboard.writeText(url)
                setCopied(true)
                setTimeout(() => setCopied(false), 1500)
              }}
              className="p-2 rounded-md border border-white/[0.1] text-gray-400 hover:text-white hover:border-white/[0.2] transition-colors shrink-0"
            >
              {copied ? <Check size={12} /> : <Copy size={12} />}
            </button>
          </div>
        ) : (
          <p className="text-[11px] text-gray-500">
            Save the workflow first — the URL includes its id.
          </p>
        )}
        <p className="text-[11px] text-gray-500 mt-1.5">
          Local machine only, and only while the app is open. Exposing it is a deliberate, separate
          step.
        </p>
      </div>
      <div>
        <label className="text-[13px] text-gray-400 font-medium block mb-2">Method</label>
        <SelectPicker
          value={config.method}
          options={[
            { value: 'POST', label: 'POST' },
            { value: 'GET', label: 'GET' }
          ]}
          onChange={(v) => onChange({ ...config, method: v as 'POST' | 'GET' })}
          variant="form"
        />
        <p className="text-[11px] text-gray-500 mt-1.5">
          The request lands as {'{{trigger.body.*}}'}, {'{{trigger.headers.*}}'}, and{' '}
          {'{{trigger.query.*}}'}.
        </p>
      </div>
    </>
  )
}
