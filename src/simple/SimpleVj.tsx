import React, { useEffect, useMemo, useRef, useState } from 'react';
import {
  fetchPreview,
  getSettings,
  hostInfo,
  listDisplays,
  openOutputWindow,
  pollSources,
  selectSource,
  setOscPort,
  setSettings,
} from './client';
import {
  RESOLUTION_PRESETS,
  defaultSettings,
  emptyPreview,
  groupSources,
  previewFromSource,
  sanitizeSettings,
  type DisplayInfo,
  type FitMode,
  type HostInfo,
  type OscListenStatus,
  type OutputSettings,
  type SimplePreview,
  type SimpleSource,
} from './model';
import { OutputCanvas } from './OutputCanvas';

const SELECTED_KEY = 'kf-simple-selected';

interface SimpleVjProps {
  onAdvanced: () => void;
}

export const SimpleVj: React.FC<SimpleVjProps> = ({ onAdvanced }) => {
  const [sources, setSources] = useState<SimpleSource[]>([]);
  const [osc, setOsc] = useState<OscListenStatus | null>(null);
  const [host, setHost] = useState<HostInfo | null>(null);
  const [ndiRuntime, setNdiRuntime] = useState(false);
  const [displays, setDisplays] = useState<DisplayInfo[]>([]);
  const [settings, setLocalSettings] = useState<OutputSettings>(defaultSettings);
  const [selectedId, setSelectedId] = useState<string | null>(() => {
    try {
      return localStorage.getItem(SELECTED_KEY);
    } catch {
      return null;
    }
  });
  const [preview, setPreview] = useState<SimplePreview>(emptyPreview);
  const [message, setMessage] = useState('');
  const [portDraft, setPortDraft] = useState('9000');

  useEffect(() => {
    let alive = true;
    void (async () => {
      const [info, initialSettings, savedDisplays] = await Promise.all([
        hostInfo(),
        getSettings().catch(() => defaultSettings()),
        listDisplays().catch(() => []),
      ]);
      if (!alive) return;
      setHost(info);
      setLocalSettings(initialSettings);
      setDisplays(savedDisplays);
    })();
    return () => {
      alive = false;
    };
  }, []);

  useEffect(() => {
    let alive = true;
    const pull = async () => {
      try {
        const polled = await pollSources();
        if (!alive) return;
        setSources(polled.sources);
        setOsc(polled.osc);
        setHost(polled.host);
        setNdiRuntime(polled.ndi_runtime);
        setPortDraft(String(polled.osc.requested_port || 9000));
      } catch (err) {
        if (!alive) return;
        setMessage(err instanceof Error ? err.message : 'Could not refresh sources.');
      }
    };
    void pull();
    const id = window.setInterval(() => void pull(), 1000);
    return () => {
      alive = false;
      window.clearInterval(id);
    };
  }, []);

  const selected = sources.find((source) => source.id === selectedId) ?? null;
  const sourcesRef = useRef(sources);
  sourcesRef.current = sources;
  const appliedSelection = useRef<string | null>(null);

  useEffect(() => {
    if (!selectedId || appliedSelection.current === selectedId) return;
    const source = sources.find((row) => row.id === selectedId);
    if (!source) return;
    appliedSelection.current = selectedId;
    void selectSource(source);
  }, [selectedId, sources]);

  useEffect(() => {
    let alive = true;
    const pull = async () => {
      const current = sourcesRef.current.find((row) => row.id === selectedId) ?? null;
      const frame = await fetchPreview(960, 540).catch(() => previewFromSource(current));
      if (alive) setPreview(frame);
    };
    void pull();
    const id = window.setInterval(() => void pull(), 120);
    return () => {
      alive = false;
      window.clearInterval(id);
    };
  }, [selectedId]);

  const groups = useMemo(() => groupSources(sources), [sources]);
  const previewPixels = useMemo(() => {
    const scale = Math.min(1, 960 / Math.max(settings.width, 1));
    return {
      width: Math.max(160, Math.round(settings.width * scale)),
      height: Math.max(90, Math.round(settings.height * scale)),
    };
  }, [settings.width, settings.height]);

  const choose = (source: SimpleSource) => {
    appliedSelection.current = source.id;
    setSelectedId(source.id);
    try {
      localStorage.setItem(SELECTED_KEY, source.id);
    } catch {
      /* ignore */
    }
    void selectSource(source);
  };

  const commitSettings = (next: OutputSettings) => {
    const clean = sanitizeSettings(next);
    setLocalSettings(clean);
    void setSettings(clean);
  };

  const commitPort = () => {
    const port = Number(portDraft);
    if (!Number.isFinite(port)) return;
    void setOscPort(port).then((status) => {
      setOsc(status);
      setPortDraft(String(status.requested_port));
    });
  };

  const resolutionValue = RESOLUTION_PRESETS.some(
    (preset) => preset.width === settings.width && preset.height === settings.height
  )
    ? `${settings.width}x${settings.height}`
    : 'custom';

  return (
    <div className="h-screen w-screen flex flex-col bg-studio-bg text-studio-text">
      <header className="h-14 shrink-0 px-6 flex items-center justify-between border-b border-studio-border">
        <div>
          <div className="text-sm font-medium tracking-tight">Kontrolfreak</div>
          <div className="text-xs text-studio-muted">Pick a source. It goes to the output.</div>
        </div>
        <button
          type="button"
          onClick={onAdvanced}
          className="text-sm px-3 py-1.5 rounded-md border border-studio-border text-studio-muted hover:text-studio-text hover:bg-studio-elevated"
        >
          Advanced
        </button>
      </header>

      <div className="flex-1 min-h-0 grid grid-cols-[300px_minmax(0,1fr)]">
        <aside className="border-r border-studio-border overflow-y-auto px-4 py-4 space-y-6">
          <SourceGroup
            title="OSC"
            hint={oscHint(osc)}
            sources={groups.osc}
            selectedId={selectedId}
            onSelect={choose}
            empty="Nothing yet. Send OSC to the port above, or wait for an mDNS advertisement."
            extra={
              <label className="flex items-center gap-2 text-xs text-studio-muted">
                Port
                <input
                  value={portDraft}
                  onChange={(event) => setPortDraft(event.target.value)}
                  onBlur={commitPort}
                  onKeyDown={(event) => {
                    if (event.key === 'Enter') commitPort();
                  }}
                  inputMode="numeric"
                  className="w-20 bg-studio-panel border border-studio-border rounded px-2 py-1 text-studio-text"
                  aria-label="OSC listen port"
                />
              </label>
            }
          />
          <SourceGroup
            title="MIDI"
            hint="Inputs and virtual ports"
            sources={groups.midi}
            selectedId={selectedId}
            onSelect={choose}
            empty="No MIDI inputs found."
          />
          <SourceGroup
            title="Video"
            hint={ndiRuntime ? 'NDI runtime connected' : 'NDI runtime not installed — test patterns are ready'}
            sources={groups.video}
            selectedId={selectedId}
            onSelect={choose}
            empty="No video sources."
          />
        </aside>

        <section className="min-w-0 flex items-center justify-center p-8">
          <div className="w-full max-w-5xl">
            <div className="aspect-video bg-black rounded-lg overflow-hidden border border-studio-border">
              <OutputCanvas
                preview={selected ? preview : emptyPreview()}
                settings={settings}
                pixelWidth={previewPixels.width}
                pixelHeight={previewPixels.height}
                showLabel
              />
            </div>
            <div className="mt-3 flex items-baseline justify-between gap-4 text-sm">
              <div className="min-w-0">
                <div className="truncate">{selected?.label ?? 'No source selected'}</div>
                <div className="text-xs text-studio-muted truncate">
                  {selected?.detail || 'The list refreshes on its own.'}
                </div>
              </div>
              {selected?.value_label ? (
                <div className="font-mono text-xs text-studio-muted shrink-0">{selected.value_label}</div>
              ) : null}
            </div>
          </div>
        </section>
      </div>

      <footer className="shrink-0 border-t border-studio-border px-6 py-4 flex flex-wrap items-end gap-x-8 gap-y-4">
        <label className="flex flex-col gap-1 text-xs text-studio-muted">
          Resolution
          <select
            value={resolutionValue}
            onChange={(event) => {
              const preset = RESOLUTION_PRESETS.find((item) => item.id === event.target.value);
              if (preset) commitSettings({ ...settings, width: preset.width, height: preset.height });
            }}
            className="bg-studio-panel border border-studio-border rounded px-2 py-1.5 text-sm text-studio-text"
          >
            {RESOLUTION_PRESETS.map((preset) => (
              <option key={preset.id} value={preset.id}>
                {preset.label}
              </option>
            ))}
            <option value="custom">Custom</option>
          </select>
        </label>
        {resolutionValue === 'custom' && (
          <div className="flex items-end gap-2 text-xs text-studio-muted">
            <label className="flex flex-col gap-1">
              Width
              <input
                type="number"
                value={settings.width}
                min={160}
                max={7680}
                onChange={(event) => commitSettings({ ...settings, width: Number(event.target.value) })}
                className="w-24 bg-studio-panel border border-studio-border rounded px-2 py-1.5 text-sm text-studio-text"
              />
            </label>
            <label className="flex flex-col gap-1">
              Height
              <input
                type="number"
                value={settings.height}
                min={90}
                max={4320}
                onChange={(event) => commitSettings({ ...settings, height: Number(event.target.value) })}
                className="w-24 bg-studio-panel border border-studio-border rounded px-2 py-1.5 text-sm text-studio-text"
              />
            </label>
          </div>
        )}

        <div className="flex flex-col gap-1 text-xs text-studio-muted">
          Fit
          <div className="flex gap-1">
            {(['contain', 'cover', 'stretch'] as FitMode[]).map((fit) => (
              <button
                key={fit}
                type="button"
                onClick={() => commitSettings({ ...settings, fit })}
                className={`px-3 py-1.5 rounded text-sm capitalize ${
                  settings.fit === fit
                    ? 'bg-studio-elevated text-studio-text border border-studio-border'
                    : 'text-studio-muted hover:text-studio-text'
                }`}
              >
                {fit}
              </button>
            ))}
          </div>
        </div>

        <div className="flex flex-col gap-1 text-xs text-studio-muted min-w-[220px] flex-1">
          Crop
          <div className="grid grid-cols-4 gap-2">
            {(
              [
                ['crop_x', 'X'],
                ['crop_y', 'Y'],
                ['crop_w', 'W'],
                ['crop_h', 'H'],
              ] as const
            ).map(([key, label]) => (
              <label key={key} className="flex flex-col gap-1">
                <span>{label}</span>
                <input
                  type="range"
                  min={0}
                  max={1}
                  step={0.01}
                  value={settings[key]}
                  aria-label={`Crop ${label}`}
                  onChange={(event) =>
                    commitSettings({ ...settings, [key]: Number(event.target.value) })
                  }
                />
              </label>
            ))}
          </div>
        </div>

        <label className="flex flex-col gap-1 text-xs text-studio-muted">
          Display
          <select
            value={settings.display_index ?? ''}
            onFocus={() => {
              void listDisplays()
                .then(setDisplays)
                .catch(() => undefined);
            }}
            onChange={(event) => {
              const value = event.target.value;
              commitSettings({
                ...settings,
                display_index: value === '' ? null : Number(value),
              });
            }}
            className="bg-studio-panel border border-studio-border rounded px-2 py-1.5 text-sm text-studio-text max-w-[240px]"
          >
            <option value="">Floating window</option>
            {displays.map((display) => (
              <option key={display.index} value={display.index}>
                {display.name} · {display.width}×{display.height}
              </option>
            ))}
          </select>
        </label>

        <button
          type="button"
          onClick={() => {
            void openOutputWindow(settings).then((status) => setMessage(status.message));
          }}
          className="px-4 py-2 rounded-md bg-studio-text text-studio-bg text-sm font-medium hover:opacity-90"
        >
          Open output
        </button>
      </footer>
      {(message || host?.wayland) && (
        <div className="px-6 pb-3 text-xs text-studio-muted space-y-1">
          {host?.wayland && (
            <p>
              This session is Wayland, so the output window opens floating. Drag it to the other
              display, then press F or double-click.
            </p>
          )}
          {message && <p>{message}</p>}
        </div>
      )}
    </div>
  );
};

function oscHint(osc: OscListenStatus | null): string {
  if (!osc) return 'Listening';
  if (osc.error) return osc.error;
  const where = osc.bound_address ? `Listening ${osc.bound_address}` : `Port ${osc.requested_port}`;
  if (osc.mdns_active && !osc.mdns_fallback) return `${where} · mDNS on`;
  if (osc.mdns_active && osc.mdns_fallback) return `${where} · mDNS partial`;
  return `${where} · mDNS off`;
}

interface SourceGroupProps {
  title: string;
  hint: string;
  sources: SimpleSource[];
  selectedId: string | null;
  onSelect: (source: SimpleSource) => void;
  empty: string;
  extra?: React.ReactNode;
}

const SourceGroup: React.FC<SourceGroupProps> = ({
  title,
  hint,
  sources,
  selectedId,
  onSelect,
  empty,
  extra,
}) => (
  <section>
    <div className="flex items-center justify-between gap-3 mb-2">
      <h2 className="text-xs uppercase tracking-wider text-studio-muted">{title}</h2>
      {extra}
    </div>
    <p className="text-xs text-studio-muted mb-2 leading-relaxed">{hint}</p>
    {sources.length === 0 ? (
      <p className="text-sm text-studio-muted/80 leading-relaxed">{empty}</p>
    ) : (
      <ul className="space-y-1">
        {sources.map((source) => {
          const on = source.id === selectedId;
          return (
            <li key={source.id}>
              <button
                type="button"
                onClick={() => onSelect(source)}
                aria-pressed={on}
                className={`w-full text-left rounded-md px-3 py-2 border ${
                  on
                    ? 'border-studio-border bg-studio-elevated'
                    : 'border-transparent hover:bg-studio-panel'
                }`}
              >
                <div className="flex items-center gap-2">
                  <span
                    className={`w-1.5 h-1.5 rounded-full ${source.live ? 'bg-emerald-400' : 'bg-studio-border'}`}
                  />
                  <span className="text-sm truncate">{source.label}</span>
                </div>
                <div className="mt-0.5 pl-3.5 text-xs text-studio-muted truncate">{source.detail}</div>
              </button>
            </li>
          );
        })}
      </ul>
    )}
  </section>
);
