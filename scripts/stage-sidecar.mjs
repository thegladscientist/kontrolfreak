#!/usr/bin/env node
/**
 * Builds the `kf-sensor-depth` crash-isolated sidecar binary and stages it into
 * `src-tauri/binaries/kf-sensor-depth-<target-triple>[.exe]` so Tauri's `externalBin`
 * bundler packages it alongside the Kontrolfreak desktop executable.
 *
 * Usage:
 *   node scripts/stage-sidecar.mjs [--release] [--target <rust-target-triple>]
 */

import { execFileSync } from 'node:child_process';
import { chmodSync, copyFileSync, existsSync, mkdirSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const __dirname = dirname(fileURLToPath(import.meta.url));
const repoRoot = resolve(__dirname, '..');

const args = process.argv.slice(2);
let release = false;
let targetTriple = null;

for (let i = 0; i < args.length; i += 1) {
  if (args[i] === '--release') {
    release = true;
  } else if (args[i] === '--target' && args[i + 1]) {
    targetTriple = args[i + 1];
    i += 1;
  } else if (args[i].startsWith('--target=')) {
    targetTriple = args[i].slice('--target='.length);
  }
}

function detectHostTriple() {
  const rustcOut = execFileSync('rustc', ['-vV'], { encoding: 'utf8' });
  const match = rustcOut.match(/^host:\s*(\S+)/m);
  if (!match) {
    throw new Error(`Could not detect Rust host triple from \`rustc -vV\`:\n${rustcOut}`);
  }
  return match[1];
}

const hostTriple = detectHostTriple();
const effectiveTriple = targetTriple || hostTriple;
const isWindowsTarget = effectiveTriple.includes('windows');
const ext = isWindowsTarget ? '.exe' : '';

const cargoArgs = ['build', '-p', 'kf-sensor-depth'];
if (release) {
  cargoArgs.push('--release');
}
if (targetTriple) {
  cargoArgs.push('--target', targetTriple);
}

console.log(`[stage-sidecar] Running: cargo ${cargoArgs.join(' ')}`);
execFileSync('cargo', cargoArgs, {
  cwd: repoRoot,
  stdio: 'inherit',
});

const profileDir = release ? 'release' : 'debug';
const builtPath = targetTriple
  ? join(repoRoot, 'target', targetTriple, profileDir, `kf-sensor-depth${ext}`)
  : join(repoRoot, 'target', profileDir, `kf-sensor-depth${ext}`);

if (!existsSync(builtPath)) {
  throw new Error(`Expected built sidecar binary not found at: ${builtPath}`);
}

const outDir = join(repoRoot, 'src-tauri', 'binaries');
mkdirSync(outDir, { recursive: true });
const stagedPath = join(outDir, `kf-sensor-depth-${effectiveTriple}${ext}`);

copyFileSync(builtPath, stagedPath);
if (!isWindowsTarget) {
  chmodSync(stagedPath, 0o755);
}

console.log(`[stage-sidecar] Staged sidecar: ${stagedPath}`);
