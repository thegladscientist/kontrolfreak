import { fitRect } from './fit';
import type { CropRect, FitMode, PatternKind } from './model';

export interface PaintScene {
  kind: string;
  label: string;
  detail: string;
  valueNorm: number | null;
  pattern: PatternKind | null;
  image: CanvasImageSource | null;
  fit: FitMode;
  crop: CropRect;
  showLabel: boolean;
}

const FULL_CROP: CropRect = { x: 0, y: 0, w: 1, h: 1 };

export function paintScene(
  ctx: CanvasRenderingContext2D,
  width: number,
  height: number,
  scene: PaintScene,
  nowMs: number
): void {
  ctx.fillStyle = '#05070b';
  ctx.fillRect(0, 0, width, height);
  const source = scene.image ?? renderGenerated(scene, nowMs);
  if (!source) {
    drawIdle(ctx, width, height, scene.label);
    return;
  }
  const srcW = sourceWidth(source);
  const srcH = sourceHeight(source);
  const rect = fitRect(srcW, srcH, width, height, scene.fit, scene.crop);
  ctx.save();
  ctx.beginPath();
  ctx.rect(0, 0, width, height);
  ctx.clip();
  ctx.imageSmoothingEnabled = true;
  ctx.drawImage(source, rect.sx, rect.sy, rect.sw, rect.sh, rect.dx, rect.dy, rect.dw, rect.dh);
  ctx.restore();
  if (scene.showLabel && scene.kind !== 'test_pattern' && scene.kind !== 'ndi') {
    drawCaption(ctx, width, scene);
  }
}

function renderGenerated(scene: PaintScene, nowMs: number): HTMLCanvasElement | null {
  if (scene.kind === 'empty') return null;
  const canvas = document.createElement('canvas');
  canvas.width = 960;
  canvas.height = 540;
  const ctx = canvas.getContext('2d');
  if (!ctx) return null;
  if (scene.kind === 'test_pattern') {
    drawPattern(ctx, canvas.width, canvas.height, scene.pattern ?? 'smpte', nowMs);
    return canvas;
  }
  drawSignal(ctx, canvas.width, canvas.height, scene, nowMs);
  return canvas;
}

function drawPattern(
  ctx: CanvasRenderingContext2D,
  width: number,
  height: number,
  pattern: PatternKind,
  nowMs: number
): void {
  if (pattern === 'grid') {
    ctx.fillStyle = '#070b12';
    ctx.fillRect(0, 0, width, height);
    ctx.strokeStyle = 'rgba(167, 139, 250, 0.45)';
    ctx.lineWidth = 1;
    const gap = 48;
    for (let x = 0; x <= width; x += gap) {
      ctx.beginPath();
      ctx.moveTo(x, 0);
      ctx.lineTo(x, height);
      ctx.stroke();
    }
    for (let y = 0; y <= height; y += gap) {
      ctx.beginPath();
      ctx.moveTo(0, y);
      ctx.lineTo(width, y);
      ctx.stroke();
    }
    const phase = ((nowMs / 1000) % 4) / 4;
    ctx.fillStyle = 'rgba(167, 139, 250, 0.85)';
    ctx.fillRect(phase * width, 0, 8, height);
    return;
  }
  if (pattern === 'gradient') {
    const shift = (nowMs / 40) % width;
    const gradient = ctx.createLinearGradient(-shift, 0, width - shift, height);
    gradient.addColorStop(0, '#1e1b4b');
    gradient.addColorStop(0.45, '#7c3aed');
    gradient.addColorStop(1, '#0f172a');
    ctx.fillStyle = gradient;
    ctx.fillRect(0, 0, width, height);
    return;
  }
  const colors = ['#c0c0c0', '#c0c000', '#00c0c0', '#00c000', '#c000c0', '#c00000', '#0000c0'];
  const barW = width / colors.length;
  colors.forEach((color, index) => {
    ctx.fillStyle = color;
    ctx.fillRect(index * barW, 0, barW + 1, height * 0.68);
  });
  ctx.fillStyle = '#10141c';
  ctx.fillRect(0, height * 0.68, width, height * 0.32);
  const ramp = ctx.createLinearGradient(0, 0, width, 0);
  ramp.addColorStop(0, '#000');
  ramp.addColorStop(1, '#fff');
  ctx.fillStyle = ramp;
  ctx.fillRect(width * 0.05, height * 0.78, width * 0.9, height * 0.12);
}

function drawSignal(
  ctx: CanvasRenderingContext2D,
  width: number,
  height: number,
  scene: PaintScene,
  nowMs: number
): void {
  const norm = scene.valueNorm == null ? 0.35 : Math.max(0, Math.min(1, scene.valueNorm));
  const osc = scene.kind === 'osc';
  const hue = osc ? 38 : 187;
  ctx.fillStyle = '#070b12';
  ctx.fillRect(0, 0, width, height);
  const lit = height * norm;
  const gradient = ctx.createLinearGradient(0, height, 0, height - lit);
  gradient.addColorStop(0, `hsla(${hue}, 70%, 46%, 0.95)`);
  gradient.addColorStop(1, `hsla(${hue}, 80%, 62%, 0.15)`);
  ctx.fillStyle = gradient;
  ctx.fillRect(0, height - lit, width, lit);
  const pulse = 0.04 + 0.03 * Math.sin(nowMs / 700);
  ctx.fillStyle = `rgba(255,255,255,${pulse})`;
  ctx.fillRect(0, height - lit, width, 3);
}

function drawIdle(ctx: CanvasRenderingContext2D, width: number, height: number, label: string): void {
  ctx.fillStyle = '#9aa6bd';
  ctx.font = '24px ui-sans-serif, system-ui, sans-serif';
  ctx.textAlign = 'center';
  ctx.textBaseline = 'middle';
  ctx.fillText(label || 'Choose a source', width / 2, height / 2);
}

function drawCaption(ctx: CanvasRenderingContext2D, width: number, scene: PaintScene): void {
  ctx.fillStyle = 'rgba(230, 237, 247, 0.78)';
  ctx.font = '22px ui-sans-serif, system-ui, sans-serif';
  ctx.textAlign = 'left';
  ctx.textBaseline = 'top';
  ctx.fillText(scene.label, 28, 24);
  if (scene.valueNorm != null) {
    ctx.font = '18px ui-monospace, monospace';
    ctx.fillStyle = 'rgba(230, 237, 247, 0.55)';
    ctx.fillText(scene.valueNorm.toFixed(3), 28, 54);
  }
  void width;
}

function sourceWidth(source: CanvasImageSource): number {
  if (source instanceof HTMLCanvasElement) return source.width;
  if (source instanceof HTMLImageElement) return source.naturalWidth || source.width || 16;
  if (typeof HTMLVideoElement !== 'undefined' && source instanceof HTMLVideoElement) {
    return source.videoWidth || 16;
  }
  return 16;
}

function sourceHeight(source: CanvasImageSource): number {
  if (source instanceof HTMLCanvasElement) return source.height;
  if (source instanceof HTMLImageElement) return source.naturalHeight || source.height || 9;
  if (typeof HTMLVideoElement !== 'undefined' && source instanceof HTMLVideoElement) {
    return source.videoHeight || 9;
  }
  return 9;
}

export const FULL_FRAME = FULL_CROP;
