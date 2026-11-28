import { describe, expect, it } from 'vitest';
import { fitRect } from '../simple/fit';
import {
  browserSources,
  defaultSettings,
  groupSources,
  isOutputLocation,
  sanitizeSettings,
  type FitMode,
} from '../simple/model';

describe('simple VJ output fit', () => {
  const full = { x: 0, y: 0, w: 1, h: 1 };

  it('letterboxes 16:9 video into an ultra-wide frame', () => {
    const rect = fitRect(1920, 1080, 1280, 360, 'contain', full);
    expect(rect.dw).toBeCloseTo(640);
    expect(rect.dh).toBeCloseTo(360);
    expect(rect.dx).toBeCloseTo(320);
    expect(rect.dy).toBeCloseTo(0);
  });

  it('stretches the source to the output buffer', () => {
    const rect = fitRect(1920, 1080, 1280, 360, 'stretch', full);
    expect(rect.dx).toBe(0);
    expect(rect.dy).toBe(0);
    expect(rect.dw).toBe(1280);
    expect(rect.dh).toBe(360);
  });

  it('covers by scaling past the frame and crops the source first', () => {
    const cover = fitRect(1920, 1080, 512, 512, 'cover', full);
    expect(cover.dw).toBeGreaterThanOrEqual(512);
    expect(cover.dh).toBeGreaterThanOrEqual(512);
    const cropped = fitRect(1920, 1080, 1920, 1080, 'stretch', {
      x: 0.25,
      y: 0,
      w: 0.5,
      h: 1,
    });
    expect(cropped.sx).toBeCloseTo(480);
    expect(cropped.sw).toBeCloseTo(960);
  });
});

describe('simple VJ source list', () => {
  it('browser loopback offers OSC, virtual MIDI, and built-in test patterns', () => {
    const groups = groupSources(browserSources(1_000, 9000));
    expect(groups.osc.length).toBeGreaterThan(0);
    expect(groups.osc[0]?.detail).toContain('/vj/dimmer');
    expect(groups.midi.some((row) => row.detail.includes('Virtual'))).toBe(true);
    expect(groups.video.filter((row) => row.kind === 'test_pattern')).toHaveLength(3);
  });

  it('recognizes the dedicated output window', () => {
    expect(isOutputLocation('http://localhost:5173/?view=output')).toBe(true);
    expect(isOutputLocation('http://localhost:5173/#output')).toBe(true);
    expect(isOutputLocation('http://localhost:5173/', 'output')).toBe(true);
    expect(isOutputLocation('http://localhost:5173/')).toBe(false);
  });

  it('keeps resolution and crop inside a usable range', () => {
    const clean = sanitizeSettings({
      ...defaultSettings(),
      width: 10,
      height: 9000,
      fit: 'nope' as FitMode,
      crop_w: 0.01,
      crop_x: 4,
    });
    expect(clean.width).toBe(160);
    expect(clean.height).toBe(4320);
    expect(clean.fit).toBe('contain');
    expect(clean.crop_w).toBeGreaterThanOrEqual(0.05);
    expect(clean.crop_x).toBeLessThanOrEqual(1 - clean.crop_w);
  });
});
