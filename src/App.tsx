import React, { useEffect } from 'react';
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

export const App: React.FC = () => {
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
      <TopBar />

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
