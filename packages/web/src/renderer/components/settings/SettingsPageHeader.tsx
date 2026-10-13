import { ReactNode } from 'react'

export function SettingsPageHeader({
  title,
  description,
  actions
}: {
  title: string
  description: string
  actions?: ReactNode
}) {
  return (
    <div className="flex items-start justify-between mb-6">
      <div>
        <h2 className="text-xl font-semibold text-white mb-1">{title}</h2>
        <p className="text-sm text-gray-500">{description}</p>
      </div>
      {actions}
    </div>
  )
}
