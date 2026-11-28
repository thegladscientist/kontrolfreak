import React, { useEffect, useRef } from 'react';
import { cropOf, patternKindFromLabel, type OutputSettings, type SimplePreview } from './model';
import { paintScene } from './paint';

interface OutputCanvasProps {
  preview: SimplePreview;
  settings: OutputSettings;
  pixelWidth: number;
  pixelHeight: number;
  showLabel: boolean;
  className?: string;
}

export const OutputCanvas: React.FC<OutputCanvasProps> = ({
  preview,
  settings,
  pixelWidth,
  pixelHeight,
  showLabel,
  className,
}) => {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const imageRef = useRef<HTMLImageElement | null>(null);
  const sceneRef = useRef({ preview, settings, showLabel });
  sceneRef.current = { preview, settings, showLabel };

  useEffect(() => {
    if (!preview.jpeg_base64 || (preview.kind !== 'ndi' && preview.kind !== 'test_pattern')) {
      imageRef.current = null;
      return;
    }
    let cancelled = false;
    const image = new Image();
    image.onload = () => {
      if (!cancelled) imageRef.current = image;
    };
    image.src = `data:image/jpeg;base64,${preview.jpeg_base64}`;
    return () => {
      cancelled = true;
    };
  }, [preview.jpeg_base64, preview.kind]);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    if (canvas.width !== pixelWidth) canvas.width = pixelWidth;
    if (canvas.height !== pixelHeight) canvas.height = pixelHeight;
  }, [pixelWidth, pixelHeight]);

  useEffect(() => {
    let frame = 0;
    const tick = (now: number) => {
      const canvas = canvasRef.current;
      const ctx = canvas?.getContext('2d');
      if (canvas && ctx) {
        const current = sceneRef.current;
        const kind = current.preview.kind;
        paintScene(
          ctx,
          canvas.width,
          canvas.height,
          {
            kind,
            label: current.preview.label,
            detail: current.preview.detail,
            valueNorm: current.preview.value_norm,
            pattern:
              kind === 'test_pattern' && !imageRef.current
                ? patternKindFromLabel(current.preview.label)
                : null,
            image: imageRef.current,
            fit: current.settings.fit,
            crop: cropOf(current.settings),
            showLabel: current.showLabel,
          },
          now
        );
      }
      frame = window.requestAnimationFrame(tick);
    };
    frame = window.requestAnimationFrame(tick);
    return () => window.cancelAnimationFrame(frame);
  }, []);

  return <canvas ref={canvasRef} className={className ?? 'block h-full w-full'} />;
};
