import type { ConnectorConfigField } from '../../../shared/types'

/**
 * One configuration field, rendered from what the connector declared.
 *
 * Shared by the built-in connect form and the MCP tool dialog, which is why it
 * is here rather than inside either of them.
 */
export function DynamicField({
  field,
  value,
  onChange
}: {
  field: ConnectorConfigField
  value: string
  onChange: (v: string) => void
}) {
  const isSecret = field.type === 'password'
  return (
    <div>
      <label className="block text-xs text-gray-500 mb-1 flex items-center gap-1.5">
        <span>{field.label}</span>
        {field.required && <span className="text-red-400">*</span>}
        {isSecret && (
          <span className="text-[9px] text-gray-600 uppercase tracking-wider">· encrypted</span>
        )}
      </label>
      {field.type === 'select' ? (
        <select
          value={value}
          onChange={(e) => onChange(e.target.value)}
          className="w-full px-3 py-1.5 bg-white/[0.05] border border-white/[0.1] rounded-sm text-sm text-gray-200 focus:border-white/[0.2] outline-none"
        >
          <option value="">—</option>
          {(field.options || []).map((opt) => (
            <option key={opt.value} value={opt.value}>
              {opt.label}
            </option>
          ))}
        </select>
      ) : field.type === 'textarea' ? (
        <textarea
          value={value}
          onChange={(e) => onChange(e.target.value)}
          placeholder={field.placeholder}
          rows={3}
          className="w-full px-3 py-1.5 bg-white/[0.05] border border-white/[0.1] rounded-sm text-sm text-gray-200 focus:border-white/[0.2] outline-none"
        />
      ) : (
        <input
          type={field.type === 'password' ? 'password' : 'text'}
          value={value}
          onChange={(e) => onChange(e.target.value)}
          placeholder={field.placeholder}
          className="w-full px-3 py-1.5 bg-white/[0.05] border border-white/[0.1] rounded-sm text-sm text-gray-200 focus:border-white/[0.2] outline-none"
        />
      )}
      {field.description && <p className="text-[10px] text-gray-600 mt-0.5">{field.description}</p>}
    </div>
  )
}
