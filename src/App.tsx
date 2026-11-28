import React, { useEffect, useState } from 'react';
import { isOutputLocation } from './simple/model';
import { OutputStage } from './simple/OutputStage';
import { SimpleVj } from './simple/SimpleVj';
import { TopBar } from './components/TopBar';
import { LeftSidebar, registerBuiltinSidebarSections } from './components/LeftSidebar';
import { CanvasWorkspace } from './components/CanvasWorkspace';
import { RightInspector } from './components/RightInspector';
import { BottomDrawer, registerBuiltinDrawerTabs } from './components/BottomDrawer';
import { CommandPalette, registerBuiltinCommandActions } from './components/CommandPalette';
import { ToastHost } from './components/ToastHost';
import { initBuiltinNodeRegistry } from './registries';
import { registerM2VideoRtpMidiPlugin } from './plugins/m2VideoRtpMidi';
import { registerM3HardwareLearnPlugin } from './plugins/m3HardwareLearn';
import { registerM4DepthSidecarPlugin } from './plugins/m4DepthSidecar';
import { useWorkspaceStore } from './store/workspaceStore';
import { bridge } from './lib/bridge';
import { telemetryStore } from './lib/telemetryStore';

let registriesBooted = false;
export function bootBuiltinRegistries(): void {
  if (registriesBooted) return;
  registriesBooted = true;
  initBuiltinNodeRegistry();
  registerBuiltinSidebarSections();
  registerBuiltinDrawerTabs();
  registerBuiltinCommandActions();
  registerM2VideoRtpMidiPlugin();
  registerM3HardwareLearnPlugin();
  registerM4DepthSidecarPlugin();
}

bootBuiltinRegistries();

const UI_MODE_KEY = 'kf-ui-mode';

function outputDocument(): boolean {
  if (typeof window === 'undefined') return false;
  const flag = (window as Window & { __KF_VIEW__?: string }).__KF_VIEW__;
  return isOutputLocation(window.location.href, flag);
}

export const App: React.FC = () => {
  const [advanced, setAdvanced] = useState(() => {
    try {
      return sessionStorage.getItem(UI_MODE_KEY) === 'advanced';
    } catch {
      return false;
    }
  });

  if (outputDocument()) return <OutputStage />;
  if (!advanced) {
    return (
      <SimpleVj
        onAdvanced={() => {
          try {
            sessionStorage.setItem(UI_MODE_KEY, 'advanced');
          } catch {
            /* ignore */
          }
          setAdvanced(true);
        }}
      />
    );
  }
  return (
    <AdvancedStudio
      onSimple={() => {
        try {
          sessionStorage.setItem(UI_MODE_KEY, 'simple');
        } catch {
          /* ignore */
        }
        setAdvanced(false);
      }}
    />
  );
};

const AdvancedStudio: React.FC<{ onSimple: () => void }> = ({ onSimple }) => {
  const bootstrap = useWorkspaceStore((s) => s.bootstrap);

  useEffect(() => {
    let cleanupTelemetry: (() => void) | undefined;
    let active = true;

    void (async () => {
      await bootstrap();
      if (!active) return;
      cleanupTelemetry = await bridge.subscribeTelemetry((batch) => {
        telemetryStore.ingestBatch(batch);
      });
    })();

    return () => {
      active = false;
      cleanupTelemetry?.();
    };
  }, [bootstrap]);

  return (
    <div className="h-screen w-screen flex flex-col bg-studio-bg text-studio-text overflow-hidden">
      <TopBar onSimple={onSimple} />

      <div className="flex-1 flex min-h-0 overflow-hidden">
        <LeftSidebar />

        <main className="flex-1 flex flex-col min-w-0 min-h-0 overflow-hidden">
          <CanvasWorkspace />
          <BottomDrawer />
        </main>

        <RightInspector />
      </div>

      <CommandPalette />
      <ToastHost />
    </div>
  );
};

export default App;
