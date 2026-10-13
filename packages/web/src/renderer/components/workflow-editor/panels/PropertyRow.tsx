export function PropertyRow({
  label,
  children,
  action
}: {
  label: string
  children: React.ReactNode
  action?: React.ReactNode
}) {
  return (
    <div className="flex items-center min-h-[32px] py-1 group/row">
      <div className="w-[110px] shrink-0 text-[12px] text-gray-500">{label}</div>
      <div className="flex-1 min-w-0 text-[12px] text-gray-200">{children}</div>
      {action && (
        <div className="opacity-0 group-hover/row:opacity-100 transition-opacity">{action}</div>
      )}
    </div>
  )
}

export function PropertySection({
  label,
  children
}: {
  label?: string
  children: React.ReactNode
}) {
  return (
    <div className="space-y-0.5">
      {label && (
        <div className="text-[10px] text-gray-600 uppercase tracking-wider font-medium pt-2 pb-1">
          {label}
        </div>
      )}
      {children}
    </div>
  )
}
