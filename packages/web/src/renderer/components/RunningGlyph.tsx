import { memo } from 'react'

interface Props {
  size?: number
  className?: string
  'aria-label'?: string
}

type Tier = 'in' | 'out' | 'hidden'

const grid: Tier[] = [
  'hidden',
  'out',
  'out',
  'hidden',
  'out',
  'in',
  'in',
  'out',
  'out',
  'in',
  'in',
  'out',
  'hidden',
  'out',
  'out',
  'hidden'
]

// Deterministic [0,1) from integer seed — every mount renders the same
// pulse pattern, so instances across the app stay in phase.
function seeded(seed: number) {
  const s = Math.sin(seed * 12.9898 + 78.233) * 43758.5453
  return s - Math.floor(s)
}

const cells = grid.map((tier, i) => {
  const delay = (seeded(i + 1) * 1.3).toFixed(3)
  const duration = (0.95 + seeded(i + 101) * 0.8).toFixed(3)
  return {
    key: i,
    tier,
    x: (i % 4) * 4,
    y: Math.floor(i / 4) * 4,
    style:
      tier === 'hidden'
        ? { opacity: 0 }
        : {
            animation: `${tier === 'in' ? 'rg-pulse-in' : 'rg-pulse-out'} ${duration}s ease-in-out infinite both`,
            animationDelay: `${delay}s`
          }
  }
})

export const RunningGlyph = memo(function RunningGlyph({
  size = 16,
  className,
  'aria-label': ariaLabel
}: Props) {
  return (
    <svg
      xmlns="http://www.w3.org/2000/svg"
      viewBox="0 0 15 15"
      width={size}
      height={size}
      fill="currentColor"
      className={className}
      role={ariaLabel ? 'img' : 'presentation'}
      aria-label={ariaLabel}
      data-component="running-glyph"
    >
      {cells.map((c) => (
        <rect
          key={c.key}
          x={c.x}
          y={c.y}
          width="3"
          height="3"
          rx="1"
          data-tier={c.tier}
          style={c.style}
        />
      ))}
    </svg>
  )
})
