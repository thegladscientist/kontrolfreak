import React, { useEffect } from 'react';
import { Command } from 'cmdk';
import { Search } from 'lucide-react';
import {
  listCommandActions,
  listNodeDefinitions,
  registerCommandAction,
} from '../registries';
import { useWorkspaceStore } from '../store/workspaceStore';

export function registerBuiltinCommandActions(): void {
  registerCommandAction({
    id: 'toggle-calm-mode',
    label: 'Toggle Calm Mode (Reduced Motion)',
    group: 'View',
    run: () => useWorkspaceStore.getState().toggleCalmMode(),
  });
  registerCommandAction({
    id: 'group-selection',
    label: 'Wrap Selected Nodes into Collapsible Rack',
    group: 'Canvas',
    shortcut: '⌘G',
    run: () => useWorkspaceStore.getState().createRackFromSelection(),
  });
  registerCommandAction({
    id: 'collapse-all-racks',
    label: 'Collapse All Racks',
    group: 'Canvas',
    run: () => useWorkspaceStore.getState().setAllRacksCollapsed(true),
  });
  registerCommandAction({
    id: 'expand-all-racks',
    label: 'Expand All Racks',
    group: 'Canvas',
    run: () => useWorkspaceStore.getState().setAllRacksCollapsed(false),
  });
  registerCommandAction({
    id: 'toggle-drawer',
    label: 'Toggle Bottom Signal Monitor Drawer',
    group: 'View',
    run: () => {
      const s = useWorkspaceStore.getState();
      s.setDrawerOpen(!s.drawerOpen);
    },
  });
  registerCommandAction({
    id: 'undo-graph',
    label: 'Undo Last Graph Edit',
    group: 'Edit',
    shortcut: '⌘Z',
    run: () => useWorkspaceStore.temporal.getState().undo(),
  });
  registerCommandAction({
    id: 'redo-graph',
    label: 'Redo Graph Edit',
    group: 'Edit',
    shortcut: '⇧⌘Z',
    run: () => useWorkspaceStore.temporal.getState().redo(),
  });
}

export const CommandPalette: React.FC = () => {
  const open = useWorkspaceStore((s) => s.commandPaletteOpen);
  const setOpen = useWorkspaceStore((s) => s.setCommandPaletteOpen);
  const devices = useWorkspaceStore((s) => s.graph.devices);
  const addDevice = useWorkspaceStore((s) => s.addDevice);
  const setSelection = useWorkspaceStore((s) => s.setSelection);

  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 'k') {
        e.preventDefault();
        setOpen(!useWorkspaceStore.getState().commandPaletteOpen);
      } else if (e.key === 'Escape' && useWorkspaceStore.getState().commandPaletteOpen) {
        setOpen(false);
      }
    };
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, [setOpen]);

  if (!open) return null;

  const actions = listCommandActions();
  const nodeDefs = listNodeDefinitions();

  return (
    <div
      className="fixed inset-0 z-50 bg-black/60 backdrop-blur-sm flex items-start justify-center pt-24 px-4"
      onClick={() => setOpen(false)}
    >
      <div
        className="w-full max-w-lg rounded-xl bg-studio-surface border border-studio-border shadow-2xl overflow-hidden"
        onClick={(e) => e.stopPropagation()}
      >
        <Command label="Kontrolfreak Command Palette" className="w-full">
          <div className="flex items-center gap-2 px-3 py-2.5 border-b border-studio-border">
            <Search className="w-4 h-4 text-studio-muted" />
            <Command.Input
              autoFocus
              placeholder="Jump to node, spawn protocol, or run action..."
              className="w-full bg-transparent text-sm text-studio-text placeholder:text-studio-muted outline-none"
            />
          </div>

          <Command.List className="max-h-80 overflow-y-auto p-2 space-y-2">
            <Command.Empty className="py-6 text-center text-xs text-studio-muted">
              No matching commands or nodes.
            </Command.Empty>

            <Command.Group heading="Actions" className="text-[10px] uppercase text-studio-muted px-1">
              {actions.map((act) => (
                <Command.Item
                  key={act.id}
                  onSelect={() => {
                    void act.run();
                    setOpen(false);
                  }}
                  className="flex items-center justify-between px-2.5 py-2 rounded-md text-xs text-slate-200 cursor-pointer aria-selected:bg-studio-elevated aria-selected:text-amber-300"
                >
                  <span>{act.label}</span>
                  {act.shortcut && (
                    <span className="font-mono text-[10px] text-studio-muted">{act.shortcut}</span>
                  )}
                </Command.Item>
              ))}
            </Command.Group>

            <Command.Group heading="Spawn Node" className="text-[10px] uppercase text-studio-muted px-1">
              {nodeDefs.map((def) => (
                <Command.Item
                  key={`spawn-${def.kindType}`}
                  onSelect={() => {
                    void addDevice(def.createDevice({ x: 240, y: 180 }));
                    setOpen(false);
                  }}
                  className="flex items-center justify-between px-2.5 py-2 rounded-md text-xs text-slate-200 cursor-pointer aria-selected:bg-studio-elevated aria-selected:text-cyan-300"
                >
                  <span>Add {def.label}</span>
                  <span className="font-mono text-[10px] text-studio-muted">{def.shortBadge}</span>
                </Command.Item>
              ))}
            </Command.Group>

            <Command.Group heading="Jump to Node" className="text-[10px] uppercase text-studio-muted px-1">
              {devices.map((dev) => (
                <Command.Item
                  key={`jump-${dev.id}`}
                  onSelect={() => {
                    setSelection({ kind: 'device', id: dev.id });
                    setOpen(false);
                  }}
                  className="flex items-center justify-between px-2.5 py-2 rounded-md text-xs text-slate-200 cursor-pointer aria-selected:bg-studio-elevated aria-selected:text-emerald-300"
                >
                  <span>{dev.display_name}</span>
                  <span className="font-mono text-[10px] text-studio-muted">
                    {dev.status.state === 'online' ? 'online' : 'offline'}
                  </span>
                </Command.Item>
              ))}
            </Command.Group>
          </Command.List>
        </Command>
      </div>
    </div>
  );
};
