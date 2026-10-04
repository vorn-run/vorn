/**
 * When the terminal overlay re-reads where its slots are.
 *
 * Every terminal is drawn in a fixed-position wrapper that follows a slot
 * somewhere in the React tree (see `terminal-registry.ts`). Following used to
 * mean reading every slot's rect on every frame, forever: 32 terminals cost 64
 * layout reads a frame while nothing moved, and any write that dirtied layout
 * (xterm moves its textarea with the cursor) turned those reads into a forced
 * layout, on every frame.
 *
 * A ResizeObserver alone cannot replace that. It sees a slot change size, but
 * not move: a sibling growing, a scroll, a Framer Motion spring animating
 * `transform`, a CSS transition. So the loop still exists, but it only runs
 * while something might be moving, and stops once the slots have held still
 * for a few frames. Whatever can move a slot starts it:
 *
 *   - any DOM change outside the overlay (React commits, Framer Motion writing
 *     `style` each frame, a class toggled), through one MutationObserver;
 *   - scrolling anywhere, the window resizing, fonts loading;
 *   - CSS transitions and animations starting, including a delayed transition
 *     once it starts to move, and hover and focus, which can change styles
 *     without touching the DOM;
 *   - a slot resizing, through a ResizeObserver, for the case none of the above
 *     covers (a container query, a flex sibling's intrinsic size);
 *   - the registry itself, through `request`, when a terminal's own state
 *     changes what its window shows.
 *
 * While a transition or Web Animation that contains a slot is running, the
 * loop keeps going even if a frame happened to see no movement. And a slow
 * safety check reads every slot a couple of times a second, so anything this
 * list misses is late by at most that, rather than wrong until the next click.
 */

export interface OverlaySyncDeps {
  /** The overlay's own root: changes inside it are the overlay's, not the layout's. */
  root: HTMLElement
  /** Terminals with a wrapper to place. */
  ids(): readonly string[]
  /** Place one terminal's wrapper; true when anything about it changed. */
  sync(id: string): boolean
  /** The elements whose size or position a wrapper follows. */
  slots(): readonly Element[]
}

export interface OverlaySync {
  /** Something may have moved: run the loop until it holds still. */
  request(): void
  /** Re-read which slots to watch for size changes. */
  slotsChanged(): void
  stop(): void
}

/** Frames with no movement before the loop stops. React effects and springs settle within this. */
export const QUIET_FRAMES = 6
/** How often the safety check reads every slot while the loop is idle. */
export const SAFETY_INTERVAL_MS = 500

export function startOverlaySync(deps: OverlaySyncDeps): OverlaySync {
  const win = deps.root.ownerDocument.defaultView ?? window
  const doc = deps.root.ownerDocument
  let raf = 0
  let quiet = 0
  let stopped = false

  const syncAll = (): boolean => {
    let moved = false
    for (const id of deps.ids()) {
      if (deps.sync(id)) moved = true
    }
    return moved
  }

  /** A running transition or animation on a slot or an ancestor of one. */
  const slotAnimating = (): boolean => {
    const animations = doc.getAnimations?.()
    if (!animations?.length) return false
    const slots = deps.slots()
    if (!slots.length) return false
    for (const animation of animations) {
      if (animation.playState !== 'running') continue
      const target = (animation.effect as KeyframeEffect | null)?.target
      if (!target) continue
      for (const slot of slots) {
        if (target === slot || target.contains(slot)) return true
      }
    }
    return false
  }

  const frame = (): void => {
    raf = 0
    if (stopped) return
    if (syncAll()) quiet = 0
    else quiet++
    if (quiet < QUIET_FRAMES || slotAnimating()) {
      if (quiet >= QUIET_FRAMES) quiet = QUIET_FRAMES - 1
      // A sync can request the next frame itself (a wrapper resized and asked
      // for a fit); one frame is enough.
      if (!raf) raf = win.requestAnimationFrame(frame)
    }
  }

  const request = (): void => {
    if (stopped) return
    quiet = 0
    if (!raf) raf = win.requestAnimationFrame(frame)
  }

  // DOM changes outside the overlay. Inside it, the changes are xterm drawing
  // and this module placing wrappers, and neither moves a slot.
  const mutations =
    typeof MutationObserver === 'function'
      ? new MutationObserver((records) => {
          for (const record of records) {
            if (!deps.root.contains(record.target)) {
              request()
              return
            }
          }
        })
      : null
  mutations?.observe(doc.body ?? doc.documentElement, {
    subtree: true,
    childList: true,
    characterData: true,
    attributes: true,
    attributeFilter: ['style', 'class', 'hidden', 'open', 'data-state']
  })

  const resizes = typeof ResizeObserver === 'function' ? new ResizeObserver(() => request()) : null
  const slotsChanged = (): void => {
    if (!resizes) return
    resizes.disconnect()
    for (const slot of deps.slots()) resizes.observe(slot)
  }
  slotsChanged()

  const events: [EventTarget, string][] = [
    [win, 'resize'],
    [doc, 'scroll'],
    [doc, 'transitionrun'],
    // A delayed transition starts moving after its delay, well after `transitionrun`.
    [doc, 'transitionstart'],
    [doc, 'transitionend'],
    [doc, 'animationstart'],
    [doc, 'animationend'],
    [doc, 'pointerover'],
    [doc, 'pointerout'],
    [doc, 'focusin'],
    [doc, 'focusout']
  ]
  const options = { capture: true, passive: true }
  for (const [target, type] of events) target.addEventListener(type, request, options)
  doc.fonts?.addEventListener?.('loadingdone', request)

  // Reads every slot at a slow beat while idle, so a move nothing above reported
  // is corrected within half a second instead of never.
  const safety = win.setInterval(() => {
    if (!raf && syncAll()) request()
  }, SAFETY_INTERVAL_MS)

  request()

  return {
    request,
    slotsChanged,
    stop(): void {
      stopped = true
      if (raf) win.cancelAnimationFrame(raf)
      raf = 0
      mutations?.disconnect()
      resizes?.disconnect()
      for (const [target, type] of events) target.removeEventListener(type, request, options)
      doc.fonts?.removeEventListener?.('loadingdone', request)
      win.clearInterval(safety)
    }
  }
}
