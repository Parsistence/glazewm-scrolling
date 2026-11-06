/**
 * Pure helpers for the workspace scrollbar shown in the top bar.
 *
 * The scrollbar represents a scrolling workspace's whole strip of columns
 * across the width of the bar. It highlights the viewport (the part of the
 * strip that's on-screen) and the focused column, similar to PaperWM.
 *
 * Kept free of DOM and Zebar APIs so it can be unit tested with Node.
 */

/**
 * Gets the scrollbar segments for a workspace, in bar pixels.
 *
 * Returns `null` if the workspace doesn't use the scrolling layout.
 *
 * @param {object} workspace Workspace from Zebar's GlazeWM provider.
 * @param {number} barWidth Width of the bar in pixels.
 * @returns {{
 *   columns: {id: string, left: number, width: number, isFocused: boolean}[],
 *   viewport: {left: number, width: number},
 *   isScrollable: boolean,
 * } | null}
 */
export function scrollbarSegments(workspace, barWidth) {
  const scrolling = workspace?.scrolling;
  if (!scrolling || barWidth <= 0) {
    return null;
  }

  const viewportWidth = workspace.width;

  // Strips narrower than the viewport are drawn relative to the viewport,
  // so that a single half-width column takes up half the bar.
  const total = Math.max(scrolling.stripWidth, viewportWidth, 1);
  const scale = barWidth / total;

  return {
    columns: scrolling.columns.map(column => ({
      id: column.id,
      left: column.start * scale,
      width: column.width * scale,
      isFocused: column.id === scrolling.focusedColumnId,
    })),
    viewport: {
      left: scrolling.scrollOffset * scale,
      width: Math.min(viewportWidth, total) * scale,
    },
    isScrollable: scrolling.stripWidth > viewportWidth,
  };
}

/**
 * Gets the column under a horizontal position on the scrollbar.
 *
 * Positions in a gap between columns resolve to the nearest column.
 * Returns `null` if there are no columns.
 *
 * @param {ReturnType<typeof scrollbarSegments>} segments
 * @param {number} x Position relative to the bar's left edge.
 * @returns {string | null} ID of the column.
 */
export function columnAtPosition(segments, x) {
  if (!segments || segments.columns.length === 0) {
    return null;
  }

  const distanceTo = column => {
    if (x < column.left) return column.left - x;
    if (x > column.left + column.width) return x - (column.left + column.width);
    return 0;
  };

  return segments.columns.reduce((nearest, column) =>
    distanceTo(column) < distanceTo(nearest) ? column : nearest,
  ).id;
}

/**
 * Turns mouse wheel events into discrete scroll steps.
 *
 * Wheel deltas are accumulated until they reach `threshold`, so that a
 * notched wheel (100 per notch in Chromium on Windows) scrolls one column
 * per notch, and a touchpad scrolls one column per equivalent swipe
 * distance. Down/right scrolls to the right.
 */
export class WheelStepper {
  /**
   * @param {object} [options]
   * @param {number} [options.threshold] Delta needed for one step.
   * @param {number} [options.resetAfterMs] Idle time after which a partial
   *   delta is discarded.
   */
  constructor({ threshold = 100, resetAfterMs = 250 } = {}) {
    this.threshold = threshold;
    this.resetAfterMs = resetAfterMs;
    this.accumulated = 0;
    this.lastEventTime = -Infinity;
  }

  /**
   * Feeds a wheel event's deltas.
   *
   * @param {number} deltaX
   * @param {number} deltaY
   * @param {number} time Event timestamp in milliseconds.
   * @returns {number} Number of steps to scroll (negative for left).
   */
  push(deltaX, deltaY, time) {
    if (time - this.lastEventTime > this.resetAfterMs) {
      this.accumulated = 0;
    }
    this.lastEventTime = time;

    // Use whichever axis dominates, so diagonal touchpad drift doesn't
    // cancel out intentional scrolling.
    const delta = Math.abs(deltaX) > Math.abs(deltaY) ? deltaX : deltaY;

    // Reverse direction resets the partial accumulation.
    if (Math.sign(delta) !== Math.sign(this.accumulated)) {
      this.accumulated = 0;
    }
    this.accumulated += delta;

    // `|| 0` normalizes `-0` to `0`.
    const steps = Math.trunc(this.accumulated / this.threshold) || 0;
    this.accumulated -= steps * this.threshold;
    return steps;
  }
}
