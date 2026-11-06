// Unit tests for `scrollbar.js`. Run with `node --test` from this folder.
import assert from 'node:assert/strict';
import test from 'node:test';

import {
  WheelStepper,
  columnAtPosition,
  scrollbarSegments,
} from './scrollbar.js';

/** Workspace with three half-width columns, scrolled to the last one. */
const workspace = {
  width: 1000,
  scrolling: {
    scrollOffset: 500,
    stripWidth: 1500,
    focusedColumnId: 'c',
    columns: [
      { id: 'a', start: 0, width: 500 },
      { id: 'b', start: 500, width: 500 },
      { id: 'c', start: 1000, width: 500 },
    ],
  },
};

test('segments scale the strip to the bar', () => {
  const segments = scrollbarSegments(workspace, 300);

  assert.deepEqual(segments.viewport, { left: 100, width: 200 });
  assert.deepEqual(
    segments.columns.map(({ left, width }) => [left, width]),
    [
      [0, 100],
      [100, 100],
      [200, 100],
    ],
  );
  assert.deepEqual(
    segments.columns.map(column => column.isFocused),
    [false, false, true],
  );
  assert.equal(segments.isScrollable, true);
});

test('narrow strips are drawn relative to the viewport', () => {
  const narrow = {
    width: 1000,
    scrolling: {
      scrollOffset: 0,
      stripWidth: 500,
      focusedColumnId: 'a',
      columns: [{ id: 'a', start: 0, width: 500 }],
    },
  };

  const segments = scrollbarSegments(narrow, 200);
  assert.deepEqual(segments.columns[0].width, 100);
  assert.deepEqual(segments.viewport, { left: 0, width: 200 });
  assert.equal(segments.isScrollable, false);
});

test('tiling workspaces have no segments', () => {
  assert.equal(scrollbarSegments({ width: 1000, scrolling: null }, 300), null);
  assert.equal(scrollbarSegments(undefined, 300), null);
});

test('positions resolve to the nearest column', () => {
  const segments = scrollbarSegments(workspace, 300);

  assert.equal(columnAtPosition(segments, 50), 'a');
  assert.equal(columnAtPosition(segments, 150), 'b');
  assert.equal(columnAtPosition(segments, 299), 'c');
  assert.equal(columnAtPosition(segments, 350), 'c');
  assert.equal(columnAtPosition(null, 10), null);
});

test('one wheel notch scrolls one column', () => {
  const stepper = new WheelStepper();

  assert.equal(stepper.push(0, 100, 0), 1);
  assert.equal(stepper.push(0, -100, 10), -1);
});

test('touchpad deltas accumulate into steps', () => {
  const stepper = new WheelStepper();

  assert.equal(stepper.push(0, 40, 0), 0);
  assert.equal(stepper.push(0, 40, 10), 0);
  assert.equal(stepper.push(0, 40, 20), 1);
});

test('partial deltas reset after idling or reversing', () => {
  const stepper = new WheelStepper();

  stepper.push(0, 90, 0);
  assert.equal(stepper.push(0, 20, 1000), 0);

  stepper.push(0, 90, 2000);
  assert.equal(stepper.push(0, -20, 2010), 0);
  assert.equal(stepper.push(0, -80, 2020), -1);
});

test('horizontal scrolling is used when it dominates', () => {
  const stepper = new WheelStepper();
  assert.equal(stepper.push(-100, 10, 0), -1);
});
