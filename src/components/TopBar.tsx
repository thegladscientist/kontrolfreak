import React, { useEffect, useState } from 'react';
import {
  Command,
  Download,
  Eye,
  EyeOff,
  Layers,
  Radio,
  Redo2,
  Undo2,
  Upload,
} from 'lucide-react';
import { useWorkspaceStore } from '../store/workspaceStore';
import { telemetryStore } from '../lib/telemetryStore';
import { bridge } from '../lib/bridge';

export const TopBar: React.FC = () => {
  const graph = useWorkspaceStore((s) => s.graph);
  const setProjectName = useWorkspaceStore((s) => s.setProjectName);
  const toggleCalmMode = useWorkspaceStore((s) => s.toggleCalmMode);
  const setCommandPaletteOpen = useWorkspaceStore((s) => s.setCommandPaletteOpen);
  const exportProjectJson = useWorkspaceStore((s) => s.exportProjectJson);
  const loadProjectJson = useWorkspaceStore((s) => s.loadProjectJson);
  const pushToast = useWorkspaceStore((s) => s.pushToast);

  const [editingName, setEditingName] = useState(false);
  const [nameDraft, setNameDraft] = useState(graph.name);
  const [msgRate, setMsgRate] = useState(0);

  const calmMode = graph.settings.calm_mode;

  useEffect(() => {
    setNameDraft(graph.name);
  }, [graph.name]);

  useEffect(() => {
    const id = setInterval(() => {
      setMsgRate(Math.round(telemetryStore.getTotalMsgPerSec()));
    }, 500);
    return () => clearInterval(id);
  }, []);

  const handleSaveDownload = async () => {
    const json = await exportProjectJson();
    const blob = new Blob([json], { type: 'application/json' });
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    const slug = graph.name.toLowerCase().replace(/[^a-z0-9]+/g, '-') || 'project';
    a.download = `${slug}.kfproj.json`;
    a.click();
    URL.revokeObjectURL(url);
    pushToast({
      title: 'Project exported',
      description: `Saved ${slug}.kfproj.json`,
      accent: 'emerald',
    });
  };

  const handleLoadFile = () => {
    const input = document.createElement('input');
    input.type = 'file';
    input.accept = '.json,.kfproj.json';
    input.onchange = async () => {
      const file = input.files?.[0];
      if (!file) return;
      try {
        const text = await file.text();
        await loadProjectJson(text, true);
      } catch (err) {
        pushToast({
          title: 'Failed to load project',
          description: err instanceof Error ? err.message : String(err),
          accent: 'rose',
        });
      }
    };
    input.click();
  };

  const handleUndo = () => useWorkspaceStore.temporal.getState().undo();
  const handleRedo = () => useWorkspaceStore.temporal.getState().redo();

  return (
    <header className="h-12 shrink-0 bg-studio-surface border-b border-studio-border px-3 flex items-center justify-between gap-3 select-none">
      {/* Left: Brand & Inline Project Title */}
      <div className="flex items-center gap-3 min-w-0">
        <div className="flex items-center gap-2">
          <div className="w-7 h-7 rounded-lg bg-gradient-to-br from-amber-500/30 to-cyan-500/30 border border-amber-400/40 flex items-center justify-center">
            <Layers className="w-4 h-4 text-amber-400" />
          </div>
          <span className="font-semibold tracking-tight text-sm text-studio-text">
            Kontrolfreak
          </span>
          <span className="text-[10px] font-mono uppercase px-1.5 py-0.5 rounded bg-studio-elevated text-studio-muted border border-studio-border">
            {bridge.isTauri() ? 'Desktop' : 'Loopback'}
          </span>
        </div>

        <div className="h-4 w-px bg-studio-border" />

        {editingName ? (
          <input
            autoFocus
            value={nameDraft}
            onChange={(e) => setNameDraft(e.target.value)}
            onBlur={() => {
              setProjectName(nameDraft.trim() || 'Untitled Rig');
              setEditingName(false);
            }}
            onKeyDown={(e) => {
              if (e.key === 'Enter') {
                setProjectName(nameDraft.trim() || 'Untitled Rig');
                setEditingName(false);
              }
              if (e.key === 'Escape') {
                setNameDraft(graph.name);
                setEditingName(false);
              }
            }}
            className="bg-studio-panel border border-amber-500/60 rounded px-2 py-0.5 text-sm text-studio-text outline-none w-48"
          />
        ) : (
          <button
            onClick={() => setEditingName(true)}
            title="Click to rename project"
            className="text-sm text-slate-200 hover:text-amber-300 truncate max-w-[200px] px-1.5 py-0.5 rounded hover:bg-studio-elevated transition-colors"
          >
            {graph.name}
          </button>
        )}
      </div>

      {/* Center: Undo/Redo + Save/Load */}
      <div className="flex items-center gap-1.5">
        <button
          onClick={handleUndo}
          title="Undo (Cmd/Ctrl+Z)"
          className="p-1.5 rounded-md text-slate-300 hover:text-white hover:bg-studio-elevated transition-colors"
        >
          <Undo2 className="w-4 h-4" />
        </button>
        <button
          onClick={handleRedo}
          title="Redo (Cmd/Ctrl+Shift+Z)"
          className="p-1.5 rounded-md text-slate-300 hover:text-white hover:bg-studio-elevated transition-colors"
        >
          <Redo2 className="w-4 h-4" />
        </button>

        <div className="h-4 w-px bg-studio-border mx-1" />

        <button
          onClick={handleSaveDownload}
          className="flex items-center gap-1.5 px-2.5 py-1 rounded-md text-xs font-medium bg-studio-panel hover:bg-studio-elevated border border-studio-border text-slate-200 transition-colors"
        >
          <Download className="w-3.5 h-3.5 text-amber-400" />
          Export
        </button>
        <button
          onClick={handleLoadFile}
          className="flex items-center gap-1.5 px-2.5 py-1 rounded-md text-xs font-medium bg-studio-panel hover:bg-studio-elevated border border-studio-border text-slate-200 transition-colors"
        >
          <Upload className="w-3.5 h-3.5 text-cyan-400" />
          Import
        </button>
      </div>

      {/* Right: Calm Mode, Cmd+K, Engine Rate */}
      <div className="flex items-center gap-2">
        <button
          onClick={() => void toggleCalmMode()}
          aria-pressed={calmMode}
          title="Toggle Calm Mode (reduced motion & softened live telemetry)"
          className={`flex items-center gap-1.5 px-2.5 py-1 rounded-md text-xs font-medium border transition-colors ${
            calmMode
              ? 'bg-cyan-500/20 border-cyan-400/50 text-cyan-200'
              : 'bg-studio-panel border-studio-border text-slate-300 hover:bg-studio-elevated'
          }`}
        >
          {calmMode ? <EyeOff className="w-3.5 h-3.5" /> : <Eye className="w-3.5 h-3.5" />}
          Calm Mode: {calmMode ? 'On' : 'Off'}
        </button>

        <button
          onClick={() => setCommandPaletteOpen(true)}
          className="flex items-center gap-1.5 px-2.5 py-1 rounded-md text-xs bg-studio-panel hover:bg-studio-elevated border border-studio-border text-slate-300 transition-colors"
        >
          <Command className="w-3.5 h-3.5" />
          <span className="font-mono text-[11px]">⌘K</span>
        </button>

        <div
          className="flex items-center gap-1.5 px-2 py-1 rounded-md bg-studio-panel border border-studio-border text-[11px] font-mono text-emerald-300"
          title="Data Plane telemetry rate (30 Hz batched)"
        >
          <Radio className={`w-3.5 h-3.5 ${calmMode ? 'opacity-70' : 'animate-pulse'}`} />
          <span>{msgRate} msg/s</span>
        </div>
      </div>
    </header>
  );
};
