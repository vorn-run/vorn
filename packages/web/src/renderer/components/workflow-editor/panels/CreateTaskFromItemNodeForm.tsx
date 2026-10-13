import type { CreateTaskFromItemConfig, TaskStatus } from '../../../../shared/types'
import { useAppStore } from '../../../stores'
import { SelectPicker } from '../../SelectPicker'

interface Props {
  config: CreateTaskFromItemConfig
  onChange: (config: CreateTaskFromItemConfig) => void
}

const STATUS_OPTIONS: Array<{ value: TaskStatus; label: string }> = [
  { value: 'todo', label: 'To Do' },
  { value: 'in_progress', label: 'In Progress' },
  { value: 'in_review', label: 'In Review' },
  { value: 'done', label: 'Done' },
  { value: 'cancelled', label: 'Cancelled' }
]

export function CreateTaskFromItemNodeForm({ config, onChange }: Props) {
  const projects = useAppStore((s) => s.config?.projects ?? [])

  const projectOptions: Array<{ value: string; label: string }> = [
    { value: 'fromConnection', label: 'From connection' },
    ...projects.map((p) => ({ value: p.name, label: p.name }))
  ]

  return (
    <div className="space-y-5">
      <div>
        <label className="text-[13px] text-gray-400 font-medium block mb-2">Project</label>
        <SelectPicker
          value={config.project}
          options={projectOptions}
          onChange={(v) => onChange({ ...config, project: v })}
          variant="form"
        />
        <p className="text-[11px] text-gray-500 mt-1.5">
          "From connection" uses the executionProject set on the source connection.
        </p>
      </div>

      <div>
        <label className="text-[13px] text-gray-400 font-medium block mb-2">Initial Status</label>
        <SelectPicker
          value={config.initialStatus}
          options={STATUS_OPTIONS}
          onChange={(v) => onChange({ ...config, initialStatus: v as TaskStatus })}
          variant="form"
        />
        <p className="text-[11px] text-gray-500 mt-1.5">
          Applied on first import. Local status edits are never overwritten by re-sync.
        </p>
      </div>
    </div>
  )
}
