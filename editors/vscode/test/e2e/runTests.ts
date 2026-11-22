// Runs the extension tests in VS Code twice: in a trusted workspace and in an untrusted one.
//
// `@vscode/test-electron`'s `runTests` always passes `--disable-workspace-trust`, so this script
// only borrows its download and launches VS Code itself.
//
// Environment:
// - `DS_PATH`: the `ds` executable to test (default: target/debug/ds of this repository).
// - `VSCODE_EXECUTABLE_PATH`: a VS Code to run instead of downloading the latest stable one.

import { spawn } from 'node:child_process';
import * as fs from 'node:fs';
import * as os from 'node:os';
import * as path from 'node:path';
import { downloadAndUnzipVSCode } from '@vscode/test-electron';

const extensionRoot = path.resolve(__dirname, '../../..');
const repoRoot = path.resolve(extensionRoot, '../..');

export type Mode = 'trusted' | 'untrusted';

function dsPath(): string {
  const exe = process.platform === 'win32' ? 'ds.exe' : 'ds';
  const ds = process.env.DS_PATH ?? path.join(repoRoot, 'target', 'debug', exe);
  if (!fs.existsSync(ds)) {
    throw new Error(`${ds} does not exist; build it with \`cargo build -p datascript-cli\``);
  }
  return path.resolve(ds);
}

/** A fresh workspace, user data directory and extensions directory for one run. */
function prepare(mode: Mode, ds: string): { workspace: string; userData: string; extensions: string } {
  // Short: VS Code's IPC socket lives in the user data directory, and macOS limits socket paths
  // to 103 bytes.
  const base = fs.mkdtempSync(path.join(os.tmpdir(), `ds-${mode[0]}-`));
  const workspace = path.join(base, 'workspace');
  fs.cpSync(path.join(extensionRoot, 'test', 'workspace'), workspace, { recursive: true });
  if (mode === 'untrusted') {
    // A hostile workspace setting. VS Code must ignore it in an untrusted workspace
    // (`restrictedConfigurations`), so the server still starts from the user setting.
    fs.mkdirSync(path.join(workspace, '.vscode'));
    fs.writeFileSync(
      path.join(workspace, '.vscode', 'settings.json'),
      JSON.stringify({ 'datascript.server.path': './not-ds', 'datascript.server.extraArgs': ['x'] }),
    );
  }
  const userData = path.join(base, 'u');
  fs.mkdirSync(path.join(userData, 'User'), { recursive: true });
  fs.writeFileSync(
    path.join(userData, 'User', 'settings.json'),
    JSON.stringify(
      {
        'datascript.server.path': ds,
        'security.workspace.trust.enabled': mode === 'untrusted',
        // Open the untrusted folder in Restricted Mode without asking.
        'security.workspace.trust.startupPrompt': 'never',
        'security.workspace.trust.banner': 'never',
        'extensions.autoUpdate': false,
        'telemetry.telemetryLevel': 'off',
      },
      null,
      2,
    ),
  );
  const extensions = path.join(base, 'x');
  fs.mkdirSync(extensions);
  return { workspace, userData, extensions };
}

function run(executable: string, mode: Mode, ds: string): Promise<number> {
  const { workspace, userData, extensions } = prepare(mode, ds);
  const args = [
    workspace,
    '--no-sandbox',
    '--disable-gpu-sandbox',
    '--disable-updates',
    '--skip-welcome',
    '--skip-release-notes',
    '--disable-extensions',
    ...(mode === 'trusted' ? ['--disable-workspace-trust'] : []),
    `--user-data-dir=${userData}`,
    `--extensions-dir=${extensions}`,
    `--extensionDevelopmentPath=${extensionRoot}`,
    `--extensionTestsPath=${path.join(__dirname, 'suite', 'index.js')}`,
  ];
  console.log(`\n=== ${mode} workspace: ${workspace}`);
  return new Promise((resolve, reject) => {
    const child = spawn(executable, args, {
      env: { ...process.env, DS_E2E_MODE: mode, ELECTRON_RUN_AS_NODE: undefined },
      stdio: 'inherit',
    });
    child.on('error', reject);
    child.on('exit', (code, signal) => resolve(code ?? (signal ? 1 : 0)));
  });
}

async function main(): Promise<void> {
  const ds = dsPath();
  const executable = process.env.VSCODE_EXECUTABLE_PATH ?? (await downloadAndUnzipVSCode('stable'));
  const failed: Mode[] = [];
  for (const mode of ['trusted', 'untrusted'] as const) {
    if ((await run(executable, mode, ds)) !== 0) {
      failed.push(mode);
    }
  }
  if (failed.length > 0) {
    console.error(`\nExtension tests failed in: ${failed.join(', ')}`);
    process.exit(1);
  }
}

main().catch((error) => {
  console.error(error);
  process.exit(1);
});
