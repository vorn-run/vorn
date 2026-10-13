import type { DeviceInfo } from '../../shared/types'
import { isSelectable } from './device-affordance'

export interface ListedDevice extends DeviceInfo {
  selectable: boolean
  /** Shown under the name, only when the name alone is ambiguous. */
  subtitle?: string
}

/**
 * The simulators in the order someone actually picks them.
 *
 * Booted first, because a booted simulator opens immediately while a shut-down
 * one costs a boot — and the one already running is usually the one being
 * worked on. Devices another session holds sink to the bottom: they are kept
 * visible (a missing row reads as a broken list and sends the person hunting in
 * Xcode) but they are the last thing worth scanning.
 *
 * A subtitle is added only where it settles an ambiguity. Two simulators can
 * share a name — the same model under two installed runtimes, or a duplicated
 * device — and picking blind between two identical rows is a coin toss. Where
 * the runtime tells them apart it is enough; where it does not, the head of the
 * udid does.
 */
export function orderDevices(devices: DeviceInfo[], sessionId: string): ListedDevice[] {
  const byName = new Map<string, number>()
  const byNameAndRuntime = new Map<string, number>()
  for (const d of devices) {
    byName.set(d.name, (byName.get(d.name) ?? 0) + 1)
    const key = `${d.name}\u0000${d.runtime}`
    byNameAndRuntime.set(key, (byNameAndRuntime.get(key) ?? 0) + 1)
  }

  return devices
    .map((d) => {
      const ambiguous = (byName.get(d.name) ?? 0) > 1
      const stillAmbiguous = (byNameAndRuntime.get(`${d.name}\u0000${d.runtime}`) ?? 0) > 1
      return {
        ...d,
        selectable: isSelectable(d, sessionId),
        subtitle: ambiguous
          ? stillAmbiguous
            ? `${d.runtime} · ${d.udid.slice(0, 8)}`
            : d.runtime
          : undefined
      }
    })
    .sort((a, b) => {
      if (a.selectable !== b.selectable) return a.selectable ? -1 : 1
      if (a.booted !== b.booted) return a.booted ? -1 : 1
      return a.name.localeCompare(b.name) || a.runtime.localeCompare(b.runtime)
    })
}
