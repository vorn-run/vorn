export function ToggleSwitch({
  checked,
  onChange,
  disabled,
  label
}: {
  checked: boolean
  onChange: (value: boolean) => void
  disabled?: boolean
  /** The accessible name, for a switch whose visible label is not its own element. */
  label?: string
}) {
  return (
    <button
      role="switch"
      aria-checked={checked}
      aria-label={label}
      aria-disabled={disabled}
      onClick={() => !disabled && onChange(!checked)}
      disabled={disabled}
      className={`w-10 h-6 rounded-full transition-colors relative ${
        checked ? 'bg-blue-500' : 'bg-white/[0.1]'
      } ${disabled ? 'cursor-not-allowed' : 'cursor-pointer'}`}
    >
      <div
        className={`w-4 h-4 rounded-full bg-white absolute top-1 transition-transform ${
          checked ? 'translate-x-5' : 'translate-x-1'
        }`}
      />
    </button>
  )
}
