import type React from 'react';

export interface SidebarSectionDefinition {
  id: string;
  label: string;
  order: number;
  iconName: 'layers' | 'cpu' | 'folder' | 'video' | 'radar';
  component: React.FC;
}

const sections = new Map<string, SidebarSectionDefinition>();

export function registerSidebarSection(section: SidebarSectionDefinition): void {
  sections.set(section.id, section);
}

export function listSidebarSections(): SidebarSectionDefinition[] {
  return Array.from(sections.values()).sort((a, b) => a.order - b.order);
}
