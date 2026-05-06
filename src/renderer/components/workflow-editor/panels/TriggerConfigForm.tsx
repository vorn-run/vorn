import { Zap, Clock, RefreshCw, ListPlus, ArrowRightLeft, Plug } from 'lucide-react'
import { useAppStore } from '../../../stores'
import { TriggerConfig, TaskStatus } from '../../../../shared/types'
import { SelectPicker } from '../../SelectPicker'
import { ProjectPicker } from '../../ProjectPicker'
import { ConnectorPollTriggerForm } from './ConnectorPollTriggerForm'

interface Props {
  config: TriggerConfig
  onChange: (config: TriggerConfig) => void
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

function switchTriggerType(type: TriggerConfig['triggerType']): TriggerConfig {
  switch (type) {
    case 'manual':
      return { triggerType: 'manual' }
    case 'once':
      return { triggerType: 'once', runAt: new Date().toISOString() }
    case 'recurring':
      return { triggerType: 'recurring', cron: '0 9 * * *' }
    case 'taskCreated':
      return { triggerType: 'taskCreated' }
    case 'taskStatusChanged':
      return { triggerType: 'taskStatusChanged' }
    case 'connectorPoll':
      return { triggerType: 'connectorPoll', connectionId: '', event: '', cron: '*/5 * * * *' }
  }
}

const EMPTY_PROJECTS: import('../../../../shared/types').ProjectConfig[] = []

export function TriggerConfigForm({ config, onChange }: Props) {
  const projects = useAppStore((s) => s.config?.projects ?? EMPTY_PROJECTS)

  return (
    <div className="space-y-5">
      <div>
        <label className="text-[13px] text-gray-400 font-medium block mb-2">Trigger Type</label>
        <SelectPicker
          value={config.triggerType}
          options={TRIGGER_TYPES.map(({ type, label, icon: Icon }) => ({
            value: type,
            label,
            icon: <Icon size={12} className="text-gray-400" />
          }))}
          onChange={(v) => onChange(switchTriggerType(v as TriggerConfig['triggerType']))}
          variant="form"
        />
        <p className="text-[11px] text-gray-500 mt-1.5">
          {TRIGGER_TYPES.find((t) => t.type === config.triggerType)?.hint}
        </p>
      </div>

      {config.triggerType === 'manual' &&
        (() => {
          const isContextual = config.contextual === true
          return (
            <button
              role="switch"
              aria-checked={isContextual}
              onClick={() =>
                onChange({
                  triggerType: 'manual',
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
