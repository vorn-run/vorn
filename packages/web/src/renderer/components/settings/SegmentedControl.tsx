import { ReactNode } from 'react'

interface SegmentedOption {
  value: string
  label: string
  icon?: ReactNode
  disabled?: boolean
  title?: string
}

export function SegmentedControl({
  options,
  value,
  onChange
}: {
  options: SegmentedOption[]
  value: string
  onChange: (value: string) => void
}) {
  return (
    <div role="radiogroup" className="flex bg-white/[0.04] rounded-lg p-0.5 gap-0.5">
      {options.map((opt) => (
        <button
          key={opt.value}
          role="radio"
          aria-checked={value === opt.value}
          onClick={() => !opt.disabled && onChange(opt.value)}
          disabled={opt.disabled}
          title={opt.title}
          className={`flex items-center gap-1.5 px-3 py-1 rounded-md text-xs transition-colors ${
            opt.disabled
              ? 'opacity-30 cursor-not-allowed text-gray-600'
              : value === opt.value
                ? 'bg-white/[0.1] text-white'
                : 'text-gray-500 hover:text-gray-300'
          }`}
        >
          {opt.icon}
          {opt.label}
        </button>
      ))}
    </div>
  )
}
