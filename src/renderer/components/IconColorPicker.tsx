import { Folder } from 'lucide-react'
import { PROJECT_ICON_OPTIONS, ICON_COLOR_PALETTE } from '../lib/project-icons'
import { ICON_MAP } from './project-sidebar/icon-map'

/** The icon grid and colour row, shared by anything that carries an icon. */
export function IconColorPicker({
  icon,
  color,
  onIconChange,
  onColorChange,
  showPreview = true
}: {
  icon: string
  color: string
  onIconChange: (icon: string) => void
  onColorChange: (color: string) => void
  showPreview?: boolean
}) {
  const SelectedIconComponent = ICON_MAP[icon] || Folder

  return (
    <div className="space-y-5">
      <div>
        <label className="text-xs font-medium text-gray-500 uppercase tracking-wider mb-2 block">
          Icon
        </label>
        <div className="grid grid-cols-8 gap-1.5">
          {PROJECT_ICON_OPTIONS.map((opt) => {
            const IconComp = ICON_MAP[opt.name] || Folder
            return (
              <button
                key={opt.name}
                onClick={() => onIconChange(opt.name)}
                aria-label={opt.label}
                aria-pressed={icon === opt.name}
                className={`flex items-center justify-center p-2 rounded-lg border transition-all ${
                  icon === opt.name
                    ? 'border-white/[0.2] bg-white/[0.08]'
                    : 'border-transparent hover:bg-white/[0.04]'
                }`}
                title={opt.label}
              >
                <IconComp
                  size={16}
                  color={icon === opt.name ? color : '#9ca3af'}
                  strokeWidth={1.5}
                />
              </button>
            )
          })}
        </div>
      </div>

      <div>
        <label className="text-xs font-medium text-gray-500 uppercase tracking-wider mb-2 block">
          Color
        </label>
        <div className="flex gap-2 items-center flex-wrap">
          {ICON_COLOR_PALETTE.map((swatch) => (
            <button
              key={swatch}
              onClick={() => onColorChange(swatch)}
              aria-label={`Color ${swatch}`}
              aria-pressed={color === swatch}
              className={`w-7 h-7 rounded-full border-2 transition-all ${
                color === swatch
                  ? 'border-white scale-110'
                  : 'border-transparent hover:border-white/30'
              }`}
              style={{ backgroundColor: swatch }}
            />
          ))}
          {showPreview && (
            <div className="ml-3 flex items-center gap-2">
              <SelectedIconComponent size={20} color={color} strokeWidth={1.5} />
              <span className="text-xs text-gray-500">Preview</span>
            </div>
          )}
        </div>
      </div>
    </div>
  )
}
