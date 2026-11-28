import React, { useEffect, useState } from 'react';
import { isTauriRuntime } from '../lib/bridge';
import { fetchPreview, readBrowserSnapshot, setOutputFullscreen } from './client';
import {
  SNAPSHOT_CHANNEL,
  SNAPSHOT_KEY,
  defaultSettings,
  emptyPreview,
  previewFromSource,
  type OutputSettings,
  type SimplePreview,
  type SimpleSnapshot,
} from './model';
import { OutputCanvas } from './OutputCanvas';

function sceneFromSnapshot(snapshot: SimpleSnapshot | null): {
  preview: SimplePreview;
  settings: OutputSettings;
} {
  const settings = snapshot?.settings ?? defaultSettings();
  const source = snapshot?.sources.find((row) => row.id === snapshot.selectedId) ?? null;
  return { preview: previewFromSource(source), settings };
}

export const OutputStage: React.FC = () => {
  const tauri = isTauriRuntime();
  const [preview, setPreview] = useState<SimplePreview>(emptyPreview);
  const [settings, setSettings] = useState<OutputSettings>(defaultSettings);
  const [fullscreen, setFullscreen] = useState(false);
  const [wayland, setWayland] = useState(false);

  useEffect(() => {
    if (tauri) return;
    const apply = (snapshot: SimpleSnapshot | null) => {
      const next = sceneFromSnapshot(snapshot);
      setPreview(next.preview);
      setSettings(next.settings);
      setWayland(Boolean(snapshot?.host.wayland));
    };
    apply(readBrowserSnapshot());
    let channel: BroadcastChannel | null = null;
    try {
      channel = new BroadcastChannel(SNAPSHOT_CHANNEL);
      channel.onmessage = (event: MessageEvent<SimpleSnapshot>) => apply(event.data);
    } catch {
      channel = null;
    }
    const onStorage = (event: StorageEvent) => {
      if (event.key !== SNAPSHOT_KEY || !event.newValue) return;
      try {
        apply(JSON.parse(event.newValue) as SimpleSnapshot);
      } catch {
        /* ignore malformed cache */
      }
    };
    window.addEventListener('storage', onStorage);
    return () => {
      channel?.close();
      window.removeEventListener('storage', onStorage);
    };
  }, [tauri]);

  useEffect(() => {
    if (!tauri) return;
    let alive = true;
    const pull = () => {
      const maxEdge = Math.min(settings.width, 1920);
      const maxHeight = Math.max(90, Math.round((maxEdge * settings.height) / Math.max(settings.width, 1)));
      void fetchPreview(maxEdge, maxHeight).then((frame) => {
        if (alive) setPreview(frame);
      });
    };
    pull();
    const id = window.setInterval(pull, 80);
    return () => {
      alive = false;
      window.clearInterval(id);
    };
  }, [tauri, settings.width, settings.height]);

  useEffect(() => {
    if (!tauri) return;
    let alive = true;
    const pullSettings = () => {
      void import('../lib/bridge').then(async ({ isTauriRuntime: stillTauri }) => {
        if (!stillTauri() || !alive) return;
        const { invoke } = await import('@tauri-apps/api/core');
        const [nextSettings, host] = await Promise.all([
          invoke<OutputSettings>('simple_get_settings'),
          invoke<{ wayland: boolean }>('simple_host_info'),
        ]);
        if (!alive) return;
        setSettings(nextSettings);
        setWayland(Boolean(host.wayland));
      });
    };
    pullSettings();
    const id = window.setInterval(pullSettings, 400);
    return () => {
      alive = false;
      window.clearInterval(id);
    };
  }, [tauri]);

  useEffect(() => {
    const onChange = () => setFullscreen(Boolean(document.fullscreenElement));
    document.addEventListener('fullscreenchange', onChange);
    return () => document.removeEventListener('fullscreenchange', onChange);
  }, []);

  useEffect(() => {
    if (!tauri) return;
    let alive = true;
    const sync = async () => {
      const { getCurrentWindow } = await import('@tauri-apps/api/window');
      const on = await getCurrentWindow().isFullscreen();
      if (alive) setFullscreen(on);
    };
    void sync();
    const id = window.setInterval(() => void sync(), 400);
    return () => {
      alive = false;
      window.clearInterval(id);
    };
  }, [tauri]);

  const enter = async () => {
    if (tauri) {
      await setOutputFullscreen(true);
      setFullscreen(true);
      return;
    }
    await document.documentElement.requestFullscreen().catch(() => undefined);
  };

  const leave = async () => {
    if (tauri) {
      await setOutputFullscreen(false);
      setFullscreen(false);
      return;
    }
    if (document.fullscreenElement) {
      await document.exitFullscreen().catch(() => undefined);
    }
  };

  const toggle = () => {
    if (fullscreen) void leave();
    else void enter();
  };

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === 'f' || event.key === 'F') {
        event.preventDefault();
        toggle();
      } else if (event.key === 'Escape') {
        void leave();
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  });

  const startDrag = (event: React.MouseEvent) => {
    if (!tauri || event.button !== 0) return;
    void import('@tauri-apps/api/window').then(({ getCurrentWindow }) => {
      void getCurrentWindow().startDragging();
    });
  };

  return (
    <div className="fixed inset-0 bg-black text-white" onDoubleClick={toggle}>
      <OutputCanvas
        preview={preview}
        settings={settings}
        pixelWidth={settings.width}
        pixelHeight={settings.height}
        showLabel={!fullscreen}
        className="block h-full w-full"
      />
      {!fullscreen && (
        <div
          className="absolute top-0 left-0 right-0 z-10 flex items-center justify-between gap-4 px-4 py-2 text-sm text-white/80 bg-black/55"
          onMouseDown={startDrag}
          onDoubleClick={(event) => event.stopPropagation()}
        >
          <span>
            {wayland || !tauri
              ? 'Drag this onto a display. Double-click or F for fullscreen. Esc leaves.'
              : 'Double-click or F for fullscreen. Esc leaves.'}
          </span>
          <button
            type="button"
            className="rounded border border-white/30 px-3 py-1 text-white hover:bg-white/10"
            onMouseDown={(event) => event.stopPropagation()}
            onClick={(event) => {
              event.stopPropagation();
              void enter();
            }}
          >
            Fullscreen
          </button>
        </div>
      )}
    </div>
  );
};
