import type React from 'react';

export interface DrawerTabDefinition {
  id: string;
  label: string;
  order: number;
  badge?: string;
  component: React.FC;
}

const tabs = new Map<string, DrawerTabDefinition>();

export function registerDrawerTab(tab: DrawerTabDefinition): void {
  tabs.set(tab.id, tab);
}

export function listDrawerTabs(): DrawerTabDefinition[] {
  return Array.from(tabs.values()).sort((a, b) => a.order - b.order);
}
