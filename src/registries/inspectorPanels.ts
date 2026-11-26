import type React from 'react';
import type { Device, DeviceKind, RackGroup, Wire } from '../types/kontrolfreak';

export interface InspectorExtensionPanel {
  id: string;
  title: string;
  order: number;
  /** Matches a specific DeviceKind['type'], or 'wire' / 'rack' for non-device targets. */
  target: DeviceKind['type'] | 'wire' | 'rack';
  component: React.FC<{ device?: Device; wire?: Wire; rack?: RackGroup }>;
}

const panels = new Map<string, InspectorExtensionPanel>();

export function registerInspectorPanel(panel: InspectorExtensionPanel): void {
  panels.set(panel.id, panel);
}

export function getInspectorPanelsFor(
  target: DeviceKind['type'] | 'wire' | 'rack'
): InspectorExtensionPanel[] {
  return Array.from(panels.values())
    .filter((p) => p.target === target)
    .sort((a, b) => a.order - b.order);
}
