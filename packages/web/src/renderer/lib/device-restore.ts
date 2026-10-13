import { useAppStore } from '../stores'
import { toast } from '../components/Toast'
import type { DeviceRestoreRefusal } from '../stores/types'

/**
 * Take back the simulators that were open when the app last closed.
 *
 * The claim itself lives in the store, which does not raise toasts of its own;
 * this is where a refusal becomes something the person sees. The split is worth
 * keeping: the store's job is whether the device was taken, and that has to be
 * testable without a renderer.
 */
export async function restoreDevicePanes(): Promise<void> {
  const refused = await useAppStore.getState().restoreDevicePanes()
  for (const refusal of refused) toast(describe(refusal), 'warning', { duration: 8000 })
}

/**
 * What to say about a device the launch could not take.
 *
 * Named by the reason rather than passing the message through, because the
 * message was written for someone who just asked for this device. On a launch
 * nobody asked -- the pane is being put back -- so the sentence has to start by
 * saying which device and why it is not there.
 */
function describe({ device, failure }: DeviceRestoreRefusal): string {
  switch (failure.reason) {
    case 'held-by-session':
      return `${device.name} was not restored: session ${failure.holder} is holding it.`
    case 'held-by-other-vorn':
      return `${device.name} was not restored: another Vorn (process ${failure.pid}) is driving it.`
    case 'boot-failed':
      return `${device.name} was not restored: it would not boot. ${failure.message}`
    // A device that is gone is forgotten rather than reported, so this arm is
    // unreachable through `restoreDevicePanes` -- kept so adding a reason to the
    // set fails the build here rather than falling through silently.
    case 'gone':
      return `${device.name} is no longer on this machine.`
  }
}
