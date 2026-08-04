import { Plus, Trash2 } from 'lucide-react'
import { SelectPicker } from '../../SelectPicker'
import { ensureUniqueSlug } from '../../../lib/template-vars'
import type { WorkflowInputDef, WorkflowInputType } from '../../../../shared/types'

interface Props {
  inputs: WorkflowInputDef[]
  onChange: (inputs: WorkflowInputDef[]) => void
}

const INPUT_TYPES: { value: WorkflowInputType; label: string }[] = [
  { value: 'text', label: 'Text' },
  { value: 'textarea', label: 'Long text' },
  { value: 'number', label: 'Number' },
  { value: 'select', label: 'Select' },
  { value: 'boolean', label: 'Toggle' },
  { value: 'project', label: 'Project' },
  { value: 'branch', label: 'Branch' }
]

const FIELD_CLASS =
  'w-full px-2.5 py-1.5 text-[12px] bg-white/[0.06] border border-white/[0.1] rounded-md ' +
  'text-white placeholder:text-gray-600 focus:outline-none focus:border-white/[0.2]'

/** Keys become `{{inputs.<key>}}`, so they must survive the template regex
 *  (identifier-first, no dots or spaces). */
function normalizeKey(raw: string): string {
  return raw.replace(/[^a-zA-Z0-9_]/g, '_').replace(/^(\d)/, '_$1')
}

export function WorkflowInputsEditor({ inputs, onChange }: Props) {
  // Two inputs sharing a key silently lose one of their values when the run
  // dialog builds its value map, and make `{{inputs.<key>}}` ambiguous. The
  // key is flagged rather than auto-renamed: rewriting a key mid-keystroke
  // would fight anyone typing a name that is briefly a prefix of another.
  const duplicateKeys = new Set(
    inputs.map((i) => i.key).filter((key, i, all) => key && all.indexOf(key) !== i)
  )

  const update = (index: number, patch: Partial<WorkflowInputDef>) => {
    onChange(inputs.map((input, i) => (i === index ? { ...input, ...patch } : input)))
  }

  const add = () => {
    const taken = new Set(inputs.map((i) => i.key))
    const key = ensureUniqueSlug('input', taken)
    onChange([...inputs, { key, label: '', type: 'text' }])
  }

  return (
    <div>
      <label className="text-[13px] text-gray-400 font-medium block mb-2">Run Inputs</label>
      <p className="text-[11px] text-gray-500 mb-2">
        Values the user is asked for before the run starts. Use them anywhere as{' '}
        <code className="font-mono text-gray-400">{'{{inputs.key}}'}</code>.
      </p>

      <div className="space-y-2">
        {inputs.map((input, index) => (
          <div
            key={index}
            className="p-2.5 rounded-lg border border-white/[0.06] bg-white/[0.02] space-y-2"
          >
            <div className="flex items-center gap-2">
              <input
                type="text"
                value={input.key}
                onChange={(e) => update(index, { key: normalizeKey(e.target.value) })}
                placeholder="key"
                aria-label="Input key"
                aria-invalid={duplicateKeys.has(input.key) || undefined}
                className={`${FIELD_CLASS} font-mono flex-1 ${
                  duplicateKeys.has(input.key) ? 'border-red-500/60' : ''
                }`}
              />
              <button
                onClick={() => onChange(inputs.filter((_, i) => i !== index))}
                aria-label={`Remove input ${input.key}`}
                className="p-1.5 text-gray-500 hover:text-red-400 rounded-md hover:bg-white/[0.06]
                           transition-colors shrink-0"
              >
                <Trash2 size={12} />
              </button>
            </div>

            {duplicateKeys.has(input.key) && (
              <p className="text-[11px] text-red-400">
                Duplicate key — only one input named <code className="font-mono">{input.key}</code>{' '}
                will reach the run.
              </p>
            )}

            <input
              type="text"
              value={input.label}
              onChange={(e) => update(index, { label: e.target.value })}
              placeholder="Label shown in the run dialog"
              aria-label="Input label"
              className={FIELD_CLASS}
            />

            <div className="flex items-center gap-2">
              <div className="flex-1 min-w-0">
                <SelectPicker
                  value={input.type}
                  options={INPUT_TYPES}
                  onChange={(v) => {
                    const type = v as WorkflowInputType
                    // Options only mean something for a select, and the
                    // default field is hidden for select and toggle; carrying
                    // either across a type change leaves config the editor
                    // can no longer show but the run dialog would still seed.
                    const keepsDefault = type !== 'boolean' && type !== 'select'
                    update(index, {
                      type,
                      options: type === 'select' ? (input.options ?? []) : undefined,
                      defaultValue: keepsDefault ? input.defaultValue : undefined
                    })
                  }}
                  variant="form"
                />
              </div>
              <label className="flex items-center gap-1.5 cursor-pointer shrink-0">
                <input
                  type="checkbox"
                  checked={input.required === true}
                  onChange={(e) => update(index, { required: e.target.checked || undefined })}
                  className="accent-white/80"
                />
                <span className="text-[12px] text-gray-400">Required</span>
              </label>
            </div>

            {input.type === 'select' && (
              <input
                type="text"
                value={(input.options ?? []).map((o) => o.value).join(', ')}
                onChange={(e) =>
                  update(index, {
                    options: e.target.value
                      .split(',')
                      .map((s) => s.trim())
                      .filter(Boolean)
                      .map((value) => ({ value, label: value }))
                  })
                }
                placeholder="Comma-separated choices"
                aria-label="Input choices"
                className={FIELD_CLASS}
              />
            )}

            {input.type !== 'boolean' && input.type !== 'select' && (
              <input
                type="text"
                value={input.defaultValue == null ? '' : String(input.defaultValue)}
                onChange={(e) => update(index, { defaultValue: e.target.value || undefined })}
                placeholder="Default value (optional)"
                aria-label="Input default value"
                className={FIELD_CLASS}
              />
            )}
          </div>
        ))}
      </div>

      <button
        onClick={add}
        className="mt-2 flex items-center gap-1.5 px-2.5 py-1.5 text-[12px] text-gray-400
                   hover:text-white rounded-md hover:bg-white/[0.06] transition-colors"
      >
        <Plus size={12} />
        Add input
      </button>
    </div>
  )
}
