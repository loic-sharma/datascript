import * as assert from 'node:assert/strict';
import * as fs from 'node:fs';
import * as path from 'node:path';
import * as vscode from 'vscode';
import type { DataScriptApi } from '../../../src/extension';

/** Activates the extension and waits for its server to start. */
export async function activate(): Promise<DataScriptApi> {
  const extension = vscode.extensions.getExtension<DataScriptApi>('datascript.datascript');
  assert.ok(extension, 'the extension is installed');
  const api = await extension.activate();
  await api.ready();
  assert.ok(api.client(), 'the language server started');
  return api;
}

export function workspaceFile(name: string): vscode.Uri {
  const folder = vscode.workspace.workspaceFolders?.[0];
  assert.ok(folder, 'a workspace folder is open');
  return vscode.Uri.joinPath(folder.uri, name);
}

/** Writes `text` to `name` in the workspace folder. */
export function write(name: string, text: string): vscode.Uri {
  const uri = workspaceFile(name);
  fs.mkdirSync(path.dirname(uri.fsPath), { recursive: true });
  fs.writeFileSync(uri.fsPath, text);
  return uri;
}

/** Writes and opens a script. */
export async function open(name: string, text: string): Promise<vscode.TextDocument> {
  const document = await vscode.workspace.openTextDocument(write(name, text));
  await vscode.window.showTextDocument(document);
  assert.equal(document.languageId, 'datascript');
  return document;
}

/** The diagnostic codes of `uri`, in order. */
export function codes(uri: vscode.Uri): string[] {
  return vscode.languages.getDiagnostics(uri).map((d) => {
    const code = d.code;
    return typeof code === 'object' ? String(code.value) : String(code);
  });
}

/** Waits until the codes of `uri` are `expected`, failing after `timeoutMs`. */
export function waitForCodes(
  uri: vscode.Uri,
  expected: string[],
  timeoutMs = 20_000,
): Promise<vscode.Diagnostic[]> {
  return new Promise((resolve, reject) => {
    const matches = () => JSON.stringify(codes(uri)) === JSON.stringify(expected);
    const finish = () => {
      clearTimeout(timer);
      subscription.dispose();
      resolve(vscode.languages.getDiagnostics(uri));
    };
    const timer = setTimeout(() => {
      subscription.dispose();
      const name = uri.scheme === 'file' ? path.basename(uri.fsPath) : uri.toString();
      reject(
        new Error(`${name} has [${codes(uri).join(', ')}], expected [${expected.join(', ')}]`),
      );
    }, timeoutMs);
    const subscription = vscode.languages.onDidChangeDiagnostics(() => {
      if (matches()) {
        finish();
      }
    });
    if (matches()) {
      finish();
    }
  });
}

export function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

/** The CodeLenses VS Code shows for `uri`, resolved. */
export async function codeLenses(uri: vscode.Uri): Promise<vscode.CodeLens[]> {
  const lenses = await vscode.commands.executeCommand<vscode.CodeLens[]>(
    'vscode.executeCodeLensProvider',
    uri,
  );
  return lenses ?? [];
}

/** The open terminal named `name`, waiting for it to open. */
export async function terminalNamed(name: string, timeoutMs = 10_000): Promise<vscode.Terminal> {
  const deadline = Date.now() + timeoutMs;
  for (;;) {
    const found = vscode.window.terminals.find((t) => t.name === name);
    if (found) {
      return found;
    }
    assert.ok(Date.now() < deadline, `no terminal named ${name}`);
    await sleep(20);
  }
}

/** A notification shown through `vscode.window.show*Message`. */
export interface ShownMessage {
  message: string;
  items: string[];
}

/** The `vscode.window` notification methods `captureMessages` can intercept. */
export type MessageKind = 'showInformationMessage' | 'showWarningMessage' | 'showErrorMessage';

/**
 * Intercepts `vscode.window.showInformationMessage`, `showWarningMessage` or `showErrorMessage`
 * while `body` runs, returning what was shown and always restoring the original method.
 */
export async function captureMessages(
  kind: MessageKind,
  body: (captured: ShownMessage[]) => Promise<void>,
): Promise<ShownMessage[]> {
  const captured: ShownMessage[] = [];
  const win = vscode.window as unknown as Record<
    MessageKind,
    (...args: unknown[]) => Thenable<unknown>
  >;
  const original = win[kind];
  win[kind] = (message: unknown, ...rest: unknown[]) => {
    const items = rest
      .filter((arg) => typeof arg === 'string' || (typeof arg === 'object' && arg !== null && 'title' in arg))
      .map((arg) => (typeof arg === 'string' ? arg : String((arg as { title: unknown }).title)));
    captured.push({ message: String(message), items });
    return Promise.resolve(undefined);
  };
  try {
    await body(captured);
  } finally {
    win[kind] = original;
  }
  return captured;
}

/** Updates a `datascript.server.*` workspace setting and waits for the restart it triggers. */
export async function updateServerSetting(
  api: DataScriptApi,
  key: string,
  value: unknown,
  timeoutMs = 10_000,
): Promise<void> {
  const config = vscode.workspace.getConfiguration('datascript.server');
  if (value === undefined && config.inspect(key)?.workspaceValue === undefined) {
    await api.ready();
    return;
  }
  let resolveChanged!: () => void;
  let timer: NodeJS.Timeout | undefined;
  const changed = new Promise<void>((resolve, reject) => {
    resolveChanged = resolve;
    timer = setTimeout(
      () =>
        reject(
          new Error(
            `timed out waiting for datascript.server.${key} configuration change after ${timeoutMs}ms`,
          ),
        ),
      timeoutMs,
    );
  });
  const subscription = vscode.workspace.onDidChangeConfiguration((event) => {
    if (event.affectsConfiguration('datascript.server')) {
      resolveChanged();
    }
  });
  try {
    await config.update(key, value, vscode.ConfigurationTarget.Workspace);
    await changed;
  } finally {
    clearTimeout(timer);
    subscription.dispose();
  }
  await api.ready();
}
