import type { DeviceInfo, MobileProject } from '../../shared/types'

/**
 * Whether to offer the device control for a session.
 *
 * The gate exists because a simulator button on a Rails or web repo is noise —
 * but it is deliberately generous in two directions:
 *
 * - An open device pane always wins. A heuristic that is right 95% of the time
 *   is wrong eventually, and a button that vanishes out from under a device
 *   someone is already driving is far worse than one that shouldn't be there.
 * - An unprobed project (cache miss, probe in flight, probe failed) shows
 *   nothing rather than flashing a button in and out on every mount.
 *
 * None of this gates the agent tools. An agent asked to open a simulator for
 * some other directory must still be able to, whatever this returns.
 */
export function shouldShowDeviceButton(
  mobile: MobileProject | undefined,
  hasDevicePane: boolean
): boolean {
  if (hasDevicePane) return true
  return mobile?.isMobile === true
}

/**
 * Whether this session may drive `device`.
 *
 * A device claimed by another session is shown in the picker, not hidden: the
 * person needs to see that the iPhone they expected is busy and who has it,
 * otherwise a missing row reads as a broken simulator list and they go hunting
 * in Xcode.
 */
export function isSelectable(device: DeviceInfo, sessionId: string): boolean {
  return !device.claimedBy || device.claimedBy === sessionId
}
