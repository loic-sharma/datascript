// The DataScript extension: starts `ds lsp` and connects it to VS Code.
//
// Highlighting (the TextMate grammar), snippets and the language configuration need no code; they
// work before the server starts and without it. This file starts the language client and wires
// up Run (src/run.ts): the server's "▶ Run" CodeLenses and the Run commands.
//
// Workspace Trust: in an untrusted workspace the server is started with
// `initializationOptions.syntaxOnly`, so it reports syntax errors only and never reads a file
// (no schemas). Workspace values of `datascript.server.*` are ignored there (package.json
// `restrictedConfigurations`), so an untrusted folder cannot pick the program VS Code runs. When
// the user grants trust, the server restarts with the full check. Nothing runs in an untrusted
// workspace: the CodeLenses are hidden, the commands refuse, and so does the server.
//
// The server starts in the first workspace folder, or else in the home directory: an untitled
// document resolves relative paths against the workspace folder, or else against the server's
// working directory (D-056).

import * as os from 'node:os';
import * as path from 'node:path';
import * as vscode from 'vscode';
import {
  LanguageClient,
  LanguageClientOptions,
  ServerOptions,
} from 'vscode-languageclient/node';
import { RUN_FILE_COMMAND, RUN_STATEMENT_COMMAND, RunRecord, Runner } from './run';

/** The `initializationOptions` the server reads (crates/datascript-lsp/src/server.rs). */
export interface InitializationOptions {
  /** Report lexical and syntax errors only, reading no files. */
  syntaxOnly: boolean;
}

/** What `activate` returns, for tests and other extensions. */
export interface DataScriptApi {
  /** The running language client, if the server started. */
  client(): LanguageClient | undefined;
  /** The options the running server was started with. */
  initializationOptions(): InitializationOptions | undefined;
  /** Settles once the latest start or restart has finished, successfully or not. */
  ready(): Promise<void>;
  /** The latest finished Run: what the server returned and what the terminal showed. */
  lastRun(): RunRecord | undefined;
}

let client: LanguageClient | undefined;
let options: InitializationOptions | undefined;
let output: vscode.OutputChannel | undefined;
/** Starts and stops run one after another, in the order they were asked for. */
let queue: Promise<void> = Promise.resolve();
let runner: Runner | undefined;

export function activate(context: vscode.ExtensionContext): DataScriptApi {
  output = vscode.window.createOutputChannel('DataScript');
  const run = new Runner(() => client);
  runner = run;
  context.subscriptions.push(
    output,
    run,
    vscode.commands.registerCommand('datascript.restartServer', () => restart()),
    vscode.commands.registerCommand(RUN_STATEMENT_COMMAND, (uri?: string, statement?: number) =>
      run.runStatement(uri, statement),
    ),
    vscode.commands.registerCommand(RUN_FILE_COMMAND, (uri?: string) => run.runFile(uri)),
    vscode.workspace.onDidGrantWorkspaceTrust(() => {
      output?.appendLine('The workspace is now trusted: restarting with the full check.');
      restart();
    }),
    vscode.workspace.onDidChangeConfiguration((event) => {
      if (event.affectsConfiguration('datascript.server')) {
        restart();
      }
    }),
  );
  restart();
  return {
    client: () => client,
    initializationOptions: () => options,
    ready: () => queue,
    lastRun: () => run.lastRun(),
  };
}

export function deactivate(): Promise<void> {
  return enqueue(stop);
}

function restart(): Promise<void> {
  return enqueue(async () => {
    await stop();
    await start();
  });
}

function enqueue(step: () => Promise<void>): Promise<void> {
  queue = queue.then(step, step);
  return queue;
}

async function start(): Promise<void> {
  const config = vscode.workspace.getConfiguration('datascript.server');
  const command = resolveServerPath(config.get<string>('path') || 'ds');
  const args = [...config.get<string[]>('extraArgs', []), 'lsp'];
  const startOptions: InitializationOptions = { syntaxOnly: !vscode.workspace.isTrusted };

  const cwd = vscode.workspace.workspaceFolders?.[0]?.uri.fsPath ?? os.homedir();
  const serverOptions: ServerOptions = { command, args, options: { cwd } };
  const clientOptions: LanguageClientOptions = {
    documentSelector: [
      { language: 'datascript', scheme: 'file' },
      { language: 'datascript', scheme: 'untitled' },
    ],
    initializationOptions: startOptions,
    outputChannel: output,
    middleware: {
      // No Run in an untrusted workspace (D-053). The syntax-only server returns no lenses
      // either; this also covers the moment trust changes, before the restart.
      provideCodeLenses: (document, token, next) =>
        vscode.workspace.isTrusted ? next(document, token) : [],
    },
  };
  const next = new LanguageClient('datascript', 'DataScript', serverOptions, clientOptions);
  output?.appendLine(
    `Starting ${command} ${args.join(' ')}` +
      (startOptions.syntaxOnly ? ' (untrusted workspace: syntax errors only)' : ''),
  );
  try {
    await next.start();
  } catch (error) {
    await next.dispose().catch(() => undefined);
    reportStartFailure(command, error);
    return;
  }
  client = next;
  options = startOptions;
}

async function stop(): Promise<void> {
  runner?.cancelAll();
  const current = client;
  client = undefined;
  options = undefined;
  if (current) {
    await current.dispose().catch((error) => {
      output?.appendLine(`Stopping the language server failed: ${error}`);
    });
  }
}

/**
 * The executable to run. A bare name is looked up on `PATH` by the OS; a path starting with `~`
 * is in the home directory; any other relative path is relative to the first workspace folder.
 */
export function resolveServerPath(setting: string): string {
  if (setting === '~' || setting.startsWith('~/') || setting.startsWith('~\\')) {
    return path.join(os.homedir(), setting.slice(1));
  }
  if (path.isAbsolute(setting) || !/[\\/]/.test(setting)) {
    return setting;
  }
  const folder = vscode.workspace.workspaceFolders?.[0];
  return folder ? path.join(folder.uri.fsPath, setting) : setting;
}

function reportStartFailure(command: string, error: unknown): void {
  const reason = error instanceof Error ? error.message : String(error);
  output?.appendLine(`Cannot start the language server: ${reason}`);
  const notFound = /ENOENT/.test(reason);
  const message = notFound
    ? `DataScript: cannot find the language server \`${command}\`. Install \`ds\` or set ` +
      '`datascript.server.path` to the `ds` executable. Highlighting works without it.'
    : `DataScript: the language server \`${command}\` did not start: ${reason}`;
  void vscode.window
    .showErrorMessage(message, 'Open Settings', 'Show Log')
    .then((choice) => {
      if (choice === 'Open Settings') {
        void vscode.commands.executeCommand('workbench.action.openSettings', 'datascript.server');
      } else if (choice === 'Show Log') {
        output?.show();
      }
    });
}
