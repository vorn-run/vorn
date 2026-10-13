import { GitBranch } from 'lucide-react'
import { useAppStore } from '../stores'
import { ProjectPicker } from './ProjectPicker'
import { SelectPicker } from './SelectPicker'
import { parseNumberInput } from '../lib/workflow-inputs'
import type { WorkflowInputDef } from '../../shared/types'

const INPUT_CLASS =
  'w-full px-3 py-2 text-[13px] bg-white/[0.06] border border-white/[0.1] rounded-md ' +
  'text-white placeholder-gray-600 focus:outline-none focus:border-white/[0.2]'

interface Props {
  defs: WorkflowInputDef[]
  values: Record<string, unknown>
  onChange: (key: string, value: unknown) => void
}

export function WorkflowInputFields({ defs, values, onChange }: Props) {
  const projects = useAppStore((s) => s.config?.projects)

  if (defs.length === 0) return null

  return (
    <>
      {defs.map((def) => {
        const value = values[def.key]
        const label = def.label || def.key
        return (
          <div key={def.key}>
            {def.type !== 'boolean' && (
              <label className="text-[11px] font-medium text-gray-500 uppercase tracking-wider mb-2 block">
                {label}
                {def.required && <span className="text-danger ml-1">*</span>}
              </label>
            )}

            {def.type === 'textarea' && (
              <textarea
                value={String(value ?? '')}
                onChange={(e) => onChange(def.key, e.target.value)}
                placeholder={def.placeholder}
                aria-label={label}
                rows={3}
                className={`${INPUT_CLASS} resize-y`}
              />
            )}

            {def.type === 'number' && (
              <input
                type="number"
                value={value === '' || value == null ? '' : String(value)}
                onChange={(e) => onChange(def.key, parseNumberInput(e.target.value))}
                placeholder={def.placeholder}
                aria-label={label}
                className={INPUT_CLASS}
              />
            )}

            {def.type === 'select' && (
              <SelectPicker
                value={String(value ?? '')}
                options={def.options ?? []}
                onChange={(v) => onChange(def.key, v)}
                placeholder={def.placeholder || 'Select...'}
                variant="form"
              />
            )}

            {def.type === 'boolean' && (
              <label className="flex items-center gap-2 cursor-pointer">
                <input
                  type="checkbox"
                  checked={value === true}
                  aria-label={label}
                  onChange={(e) => onChange(def.key, e.target.checked)}
                  className="accent-white/80"
                />
                <span className="text-[13px] text-gray-300">{label}</span>
              </label>
            )}

            {def.type === 'project' && (
              <ProjectPicker
                currentProject={String(value ?? '')}
                projects={projects ?? []}
                onChange={(name) => onChange(def.key, name)}
                variant="form"
                allowNone={!def.required}
              />
            )}

            {def.type === 'branch' && (
              <div className="flex items-center gap-1.5 px-3 py-2 bg-white/[0.06] border border-white/[0.1] rounded-md">
                <GitBranch size={12} className="text-gray-500 shrink-0" />
                <input
                  type="text"
                  value={String(value ?? '')}
                  onChange={(e) => onChange(def.key, e.target.value)}
                  placeholder={def.placeholder || 'branch name'}
                  aria-label={label}
                  className="flex-1 min-w-0 bg-transparent text-[13px] text-white placeholder-gray-600
                             focus:outline-none border-none px-0"
                />
              </div>
            )}

            {def.type === 'text' && (
              <input
                type="text"
                value={String(value ?? '')}
                onChange={(e) => onChange(def.key, e.target.value)}
                placeholder={def.placeholder}
                aria-label={label}
                className={INPUT_CLASS}
              />
            )}

            {def.description && <p className="text-[11px] text-gray-500 mt-1">{def.description}</p>}
          </div>
        )
      })}
    </>
  )
}
