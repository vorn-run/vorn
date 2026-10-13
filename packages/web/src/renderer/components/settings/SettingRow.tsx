import { ReactNode } from 'react'

export function SettingRow({
  label,
  description,
  note,
  children,
  disabled
}: {
  label: string
  description: string
  /**
   * A condition that changes what the description promises, shown under it.
   *
   * For settings whose behaviour something else can quietly override. Plain
   * muted text rather than a callout: this is a fact about the current state,
   * not a warning, and colour here is reserved for status.
   */
  note?: ReactNode
  children: ReactNode
  disabled?: boolean
}) {
  return (
    <div
      className={`flex items-center justify-between py-4 border-b border-white/[0.06] ${disabled ? 'opacity-40' : ''}`}
    >
      <div>
        <div className="text-sm font-medium text-gray-200">{label}</div>
        <div className="text-xs text-gray-500 mt-0.5">{description}</div>
        {note && (
          <div className="text-xs text-gray-400 mt-1.5 pl-2 border-l border-white/[0.12]">
            {note}
          </div>
        )}
      </div>
      {children}
    </div>
  )
}
