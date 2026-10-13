export function OptionRow({
  selected,
  dot,
  label,
  title,
  onClick
}: {
  selected: boolean
  dot?: string
  label: string
  title?: string
  onClick: () => void
}) {
  return (
    <button
      onClick={onClick}
      title={title}
      className={`w-full text-left px-3 py-1.5 text-[12px] transition-colors flex items-center gap-2 ${
        selected
          ? 'text-white bg-white/[0.06]'
          : 'text-gray-300 hover:text-white hover:bg-white/[0.04]'
      }`}
    >
      {selected ? (
        <svg
          width="11"
          height="11"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth="3"
        >
          <polyline points="20 6 9 17 4 12" />
        </svg>
      ) : (
        <span className="w-[11px]" />
      )}
      {dot && <span className={`w-1.5 h-1.5 rounded-full ${dot} shrink-0`} />}
      {label}
    </button>
  )
}
