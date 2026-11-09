/**
 * The conversation pane is the operator-visible transcript surface: callers
 * address `conversation-pane` when they mean "the scrollable transcript" (the
 * workbench layout keeps the real scroller one level down so the composer can
 * stay pinned). Mirror the scroll metrics so a programmatic
 * `pane.scrollTop = pane.scrollHeight` actually means "read to the bottom"
 * instead of silently no-op'ing on the non-scrolling flex wrapper.
 *
 * Structural typing keeps this testable with plain objects (no DOM required);
 * the DOM call site passes the wrapper and its scrolling child.
 */

export interface ScrollSurface {
  scrollTop: number
  readonly scrollHeight: number
  readonly clientHeight: number
}

export interface MirrorOptions {
  /**
   * Called after a programmatic `scrollTop` assignment is forwarded. The DOM
   * only emits `scroll` when the position actually changes, so an assignment
   * that lands on the value already in place — the common "pane is already at
   * the bottom, now read it" case — would otherwise be invisible to the
   * transcript. The callback lets the reader treat the assignment itself as
   * the gesture.
   */
  onScrollTop?: (value: number) => void
}

/** Forward metrics/position from `source` onto `target`; returns an undo. */
export function mirrorScrollSurface(
  target: object,
  source: ScrollSurface,
  options: MirrorOptions = {},
): () => void {
  const { onScrollTop } = options
  try {
    Object.defineProperties(target, {
      scrollHeight: { configurable: true, get: () => source.scrollHeight },
      clientHeight: { configurable: true, get: () => source.clientHeight },
      scrollTop: {
        configurable: true,
        get: () => source.scrollTop,
        set: (value: number) => {
          source.scrollTop = value
          onScrollTop?.(value)
        },
      },
    })
  } catch {
    // Locked-down DOM / already-shadowed property: keep native behaviour.
    return () => {}
  }
  return () => {
    try {
      const bag = target as Record<string, unknown>
      delete bag.scrollHeight
      delete bag.clientHeight
      delete bag.scrollTop
    } catch {
      /* best-effort restore */
    }
  }
}
