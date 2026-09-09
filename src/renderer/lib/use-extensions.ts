import { useEffect, useState } from 'react'
import type {
  ExtensionPaneContribution,
  InstalledConnectorPack,
  SdkConnectorIcon
} from '../../shared/types'

/**
 * The installed extensions, read once and shared by everything that names one.
 *
 * Activation says *which* contributions show on a card; it deliberately carries
 * ids and not titles, because what a pane is called belongs to the pack that
 * ships it and would otherwise be copied into every push. So the two are read
 * together: activation per session, and this list once per window.
 */
const EMPTY: InstalledConnectorPack[] = []

let cache: InstalledConnectorPack[] | undefined
let inFlight: Promise<InstalledConnectorPack[]> | undefined

// Asked once. A failure is not kept, so the next reader asks again.
async function load(): Promise<InstalledConnectorPack[]> {
  if (cache) return cache
  if (inFlight) return inFlight
  inFlight = Promise.resolve(window.api?.listExtensions?.())
    .then((packs) => {
      if (packs) cache = packs
      return cache ?? EMPTY
    })
    .catch(() => EMPTY)
    .finally(() => {
      inFlight = undefined
    })
  return inFlight
}

/** Read on mount, which for the menu is every time it opens. */
export function useExtensions(): InstalledConnectorPack[] {
  const [packs, setPacks] = useState<InstalledConnectorPack[]>(() => cache ?? EMPTY)

  useEffect(() => {
    let live = true
    void load().then((next) => {
      if (live) setPacks(next)
    })
    return () => {
      live = false
    }
  }, [])

  return packs
}

/** Ask again — for after an install or an uninstall changed what is on disk. */
export async function refreshExtensions(): Promise<InstalledConnectorPack[]> {
  cache = undefined
  inFlight = undefined
  return load()
}

/** The panes one extension contributes, as declared in its manifest. */
export function extensionPanes(
  packs: InstalledConnectorPack[],
  extensionId: string
): ExtensionPaneContribution[] {
  return packs.find((p) => p.id === extensionId)?.contributes?.panes ?? []
}

/**
 * What to call a pane, reading the list first if this window has not yet.
 *
 * The menu has always read it by the time anyone can click a row, but a pane
 * can also be opened by a link handler with only ids in hand, and a pane titled
 * `report` by its own id is a pane nobody put a name on. Falls back to the ids
 * rather than refusing: an awkward title beats a pane that will not open.
 */
export async function paneLabel(
  extensionId: string,
  paneId: string
): Promise<{ title: string; extensionName: string; icon?: SdkConnectorIcon }> {
  const packs = cache ?? (await load())
  const pack = packs.find((p) => p.id === extensionId)
  const pane = pack?.contributes?.panes?.find((p) => p.id === paneId)
  const icon = pane?.icon ?? pack?.icon
  return {
    title: pane?.title ?? paneId,
    extensionName: pack?.name ?? extensionId,
    ...(icon && { icon })
  }
}

/** Test seam: forget what this process has read. */
export function __resetExtensionsCacheForTests(): void {
  cache = undefined
  inFlight = undefined
}
