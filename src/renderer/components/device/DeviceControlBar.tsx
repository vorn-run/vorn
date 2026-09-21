import {
  House,
  ImageDown,
  Keyboard,
  Loader2,
  Lock,
  Minus,
  Plus,
  RotateCw,
  Scan
} from 'lucide-react'
import { Tooltip } from '../Tooltip'
import { ICON_BUTTON } from '../../lib/icon-button'

interface Props {
  onHome: () => void
  onLock: () => void
  onSave: () => void
  onRotate: () => void
  onToggleKeyboard: () => void
  onZoomIn: () => void
  onZoomOut: () => void
  onZoomFit: () => void
  onZoomActual: () => void
  typing: boolean
  saving: boolean
  rotating: boolean
  /** The current scale as a percentage of the device's own points. */
  zoomPercent: number
  fitting: boolean
  canZoomIn: boolean
  canZoomOut: boolean
}

const SIZE = 14
const DIVIDER = <span aria-hidden className="w-px h-3.5 bg-white/[0.08] mx-1" />

/**
 * The buttons a person needs when the agent is not the one driving.
 *
 * Every one of these is something the session's agent could already do through
 * its tools; what was missing was a way for the person watching the pane to do
 * it themselves without switching to Simulator.app and losing the claim.
 *
 * Presentational, like the frame: callbacks in, no state, no calls. It wraps
 * rather than hiding overflow, because this pane is deliberately narrow — a
 * second row is honest, a silently dropped control is not.
 */
export function DeviceControlBar({
  onHome,
  onLock,
  onSave,
  onRotate,
  onToggleKeyboard,
  onZoomIn,
  onZoomOut,
  onZoomFit,
  onZoomActual,
  typing,
  saving,
  rotating,
  zoomPercent,
  fitting,
  canZoomIn,
  canZoomOut
}: Props): React.ReactElement {
  return (
    <div className="flex flex-wrap items-center justify-center gap-0.5 px-2 py-1 shrink-0 border-t border-white/[0.06]">
      <Tooltip label="Home">
        <button type="button" onClick={onHome} aria-label="Press Home" className={ICON_BUTTON}>
          <House size={SIZE} strokeWidth={2} />
        </button>
      </Tooltip>
      <Tooltip label="Lock">
        <button type="button" onClick={onLock} aria-label="Press Lock" className={ICON_BUTTON}>
          <Lock size={SIZE} strokeWidth={2} />
        </button>
      </Tooltip>
      {DIVIDER}
      <Tooltip label="Save a screenshot">
        <button
          type="button"
          onClick={onSave}
          disabled={saving}
          aria-label="Save a screenshot"
          className={`${ICON_BUTTON} ${saving ? 'opacity-50 cursor-wait' : ''}`}
        >
          {saving ? (
            <Loader2 size={SIZE} strokeWidth={2} className="animate-spin" />
          ) : (
            <ImageDown size={SIZE} strokeWidth={2} />
          )}
        </button>
      </Tooltip>
      <Tooltip label={typing ? 'Stop typing on the device (Esc)' : 'Type on the device'}>
        <button
          type="button"
          onClick={onToggleKeyboard}
          aria-label={typing ? 'Stop typing on the device' : 'Type on the device'}
          aria-pressed={typing}
          className={`${ICON_BUTTON} ${typing ? 'text-sky-400 bg-white/[0.06]' : ''}`}
        >
          <Keyboard size={SIZE} strokeWidth={2} />
        </button>
      </Tooltip>
      <Tooltip label="Rotate">
        <button
          type="button"
          onClick={onRotate}
          disabled={rotating}
          aria-label="Rotate the device"
          className={`${ICON_BUTTON} ${rotating ? 'opacity-50 cursor-wait' : ''}`}
        >
          <RotateCw size={SIZE} strokeWidth={2} />
        </button>
      </Tooltip>
      {DIVIDER}
      <Tooltip label="Zoom out">
        <button
          type="button"
          onClick={onZoomOut}
          disabled={!canZoomOut}
          aria-label="Zoom out"
          className={`${ICON_BUTTON} ${canZoomOut ? '' : 'opacity-40'}`}
        >
          <Minus size={SIZE} strokeWidth={2} />
        </button>
      </Tooltip>
      <span className="text-[10px] tabular-nums text-gray-500 w-9 text-center">{zoomPercent}%</span>
      <Tooltip label="Zoom in">
        <button
          type="button"
          onClick={onZoomIn}
          disabled={!canZoomIn}
          aria-label="Zoom in"
          className={`${ICON_BUTTON} ${canZoomIn ? '' : 'opacity-40'}`}
        >
          <Plus size={SIZE} strokeWidth={2} />
        </button>
      </Tooltip>
      <Tooltip label="Fit the device to the pane">
        <button
          type="button"
          onClick={onZoomFit}
          aria-label="Fit the device to the pane"
          aria-pressed={fitting}
          className={`${ICON_BUTTON} ${fitting ? 'text-sky-400 bg-white/[0.06]' : ''}`}
        >
          <Scan size={SIZE} strokeWidth={2} />
        </button>
      </Tooltip>
      <Tooltip label="One pixel per point">
        <button
          type="button"
          onClick={onZoomActual}
          aria-label="Show the device at actual size"
          className={`${ICON_BUTTON} text-[10px] leading-none font-medium`}
        >
          1:1
        </button>
      </Tooltip>
    </div>
  )
}
