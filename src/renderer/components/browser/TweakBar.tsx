import { memo } from 'react'
import type { ArtifactManifest, ArtifactTweak } from '../../../shared/types'

/**
 * The controls a design artifact declared, drawn in the pane header.
 *
 * Generated from the file's own manifest rather than written per design: this
 * component knows nothing about budgets or accents, only about the four control
 * types a tweak can be. Adding a control to a design is editing its manifest.
 *
 * The bar lives in the pane's chrome and never in the document, which is what
 * makes an exported design a sheet rather than a sheet with a settings panel
 * bolted to the top.
 *
 * Controls are drawn here rather than reused from `settings/`. `ToggleSwitch`
 * and the rest are sized for a settings row — a 40×24 switch beside three other
 * controls would double the height of a header that has to sit above the work.
 */

interface Props {
  manifest: ArtifactManifest
  /** Current values, defaulting to what the manifest declared. */
  values: Record<string, unknown>
  onChange: (key: string, value: unknown) => void
}

const LABEL = 'text-[10.5px] text-ink-faint select-none'
const FIELD =
  'text-[11px] text-ink bg-white/[0.04] border border-white/[0.08] rounded px-1.5 py-0.5 ' +
  'focus:outline-none focus:ring-1 focus:ring-white/25'

function labelFor(key: string, tweak: ArtifactTweak): string {
  return tweak.label ?? key
}

/** A switch at toolbar scale, not settings scale. */
function Switch({
  on,
  label,
  onToggle
}: {
  on: boolean
  label: string
  onToggle: () => void
}): React.JSX.Element {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={on}
      aria-label={label}
      onClick={onToggle}
      className={`w-7 h-4 rounded-full relative transition-colors shrink-0
                  focus:outline-none focus:ring-1 focus:ring-white/25
                  ${on ? 'bg-bronzo' : 'bg-white/[0.12]'}`}
    >
      <span
        className={`absolute top-[3px] w-2.5 h-2.5 rounded-full bg-white transition-transform
                    ${on ? 'translate-x-[15px]' : 'translate-x-[3px]'}`}
      />
    </button>
  )
}

function Control({
  name,
  tweak,
  value,
  onChange
}: {
  name: string
  tweak: ArtifactTweak
  value: unknown
  onChange: (value: unknown) => void
}): React.JSX.Element {
  const label = labelFor(name, tweak)

  if (tweak.type === 'boolean') {
    return (
      <Switch
        on={typeof value === 'boolean' ? value : tweak.default}
        label={label}
        onToggle={() => onChange(!(typeof value === 'boolean' ? value : tweak.default))}
      />
    )
  }

  if (tweak.type === 'number') {
    return (
      <span className="flex items-center gap-1">
        <input
          type="number"
          aria-label={label}
          value={typeof value === 'number' ? value : tweak.default}
          min={tweak.min}
          max={tweak.max}
          step={tweak.step}
          onChange={(e) => {
            const next = Number(e.target.value)
            // A half-typed number is not a value to push into the design. The
            // input keeps the text either way; only a real number travels.
            if (Number.isFinite(next)) onChange(next)
          }}
          className={`${FIELD} w-[68px] tabular-nums`}
        />
        {tweak.unit && <span className={LABEL}>{tweak.unit}</span>}
      </span>
    )
  }

  if (tweak.type === 'select') {
    const current = typeof value === 'string' ? value : tweak.default
    return (
      <select
        aria-label={label}
        value={tweak.options.includes(current) ? current : tweak.options[0]}
        onChange={(e) => onChange(e.target.value)}
        className={FIELD}
      >
        {tweak.options.map((o) => (
          <option key={o} value={o}>
            {o}
          </option>
        ))}
      </select>
    )
  }

  // Colour. A declared set becomes swatches, because picking from the design's
  // own palette is the common case; anything else gets the native picker.
  const current = typeof value === 'string' ? value : tweak.default
  if (tweak.options?.length) {
    return (
      <span className="flex items-center gap-1" role="group" aria-label={label}>
        {tweak.options.map((c) => (
          <button
            key={c}
            type="button"
            aria-label={c}
            aria-pressed={c === current}
            onClick={() => onChange(c)}
            style={{ background: c }}
            className={`w-4 h-4 rounded-sm border border-white/20 shrink-0
                        focus:outline-none focus:ring-1 focus:ring-white/40
                        ${c === current ? 'ring-1 ring-white/70' : ''}`}
          />
        ))}
      </span>
    )
  }
  return (
    <input
      type="color"
      aria-label={label}
      value={current}
      onChange={(e) => onChange(e.target.value)}
      className="w-5 h-5 rounded-sm bg-transparent border border-white/20 shrink-0 p-0"
    />
  )
}

export const TweakBar = memo(function TweakBar({
  manifest,
  values,
  onChange
}: Props): React.JSX.Element | null {
  const tweaks = manifest.tweaks
  if (!tweaks || Object.keys(tweaks).length === 0) return null

  return (
    <div
      className="flex items-center gap-3 px-2 py-1 shrink-0 flex-wrap"
      role="group"
      aria-label="Design controls"
    >
      {Object.entries(tweaks).map(([name, tweak]) => (
        <span key={name} className="flex items-center gap-1.5">
          <span className={LABEL}>{labelFor(name, tweak)}</span>
          <Control
            name={name}
            tweak={tweak}
            value={values[name]}
            onChange={(v) => onChange(name, v)}
          />
        </span>
      ))}
    </div>
  )
})
