// Run (M3-05, D-054): runs a DataScript document, or one of its query statements, through the
// server's `datascript/run` request and prints what `ds run` would print in a terminal named
// "DataScript".
//
// The server runs its own copy of the document, which the client keeps in sync as the user
// types, so unsaved edits run without saving. A run is cancelled (`$/cancelRequest`, which stops
// the query on the server) when the user presses Ctrl+C in the terminal, closes it, or runs the
// same document again.

import * as vscode from 'vscode';
import {
  CancellationTokenSource,
  LanguageClient,
  LSPErrorCodes,
  ResponseError,
} from 'vscode-languageclient/node';

/** The params of `datascript/run` (crates/datascript-lsp/src/run.rs). */
export interface RunParams {
  textDocument: { uri: string };
  /** 1-based, counting query statements only, like `ds run --statement`. */
  statement?: number;
  maxRows?: number;
  color?: boolean;
}

/** The result of `datascript/run`. */
export interface RunResult {
  /** What `ds run --format table` prints for the run, `\n` line ends. */
  output: string;
  /** Errors from checking, or a runtime error. */
  failed: boolean;
}

/** How a run ended, as the API reports it to tests. */
export interface RunRecord {
  uri: string;
  statement?: number;
  /** `ok`, `failed` (errors in the program), `cancelled`, or `refused` (the server said no). */
  outcome: 'ok' | 'failed' | 'cancelled' | 'refused';
  /** The server's output, or the reason it refused. */
  output: string;
  /** Everything this run wrote to the terminal: header, output and footer, `\n` line ends. */
  terminalText: string;
}

const RUN_METHOD = 'datascript/run';
const TERMINAL_NAME = 'DataScript';

/** The commands the server's CodeLenses invoke (crates/datascript-lsp/src/run.rs). */
export const RUN_STATEMENT_COMMAND = 'datascript.runStatement';
export const RUN_FILE_COMMAND = 'datascript.runFile';

/** A terminal that shows run output and turns Ctrl+C into a cancellation. */
class RunTerminal implements vscode.Pseudoterminal {
  private readonly writer = new vscode.EventEmitter<string>();
  private readonly closer = new vscode.EventEmitter<void>();
  readonly onDidWrite = this.writer.event;
  readonly onDidClose = this.closer.event;
  /** Text written before VS Code opened the terminal. */
  private pending: string[] | undefined = [];
  readonly terminal: vscode.Terminal;

  constructor(
    private readonly onInterrupt: () => void,
    private readonly onClosed: () => void,
  ) {
    this.terminal = vscode.window.createTerminal({ name: TERMINAL_NAME, pty: this });
  }

  open(): void {
    const pending = this.pending ?? [];
    this.pending = undefined;
    for (const text of pending) {
      this.writer.fire(text);
    }
  }

  close(): void {
    this.onClosed();
  }

  handleInput(data: string): void {
    if (data.includes('\x03')) {
      this.onInterrupt();
    }
  }

  /** Writes `text`, turning `\n` into the `\r\n` a terminal needs. */
  write(text: string): void {
    const converted = text.replace(/\r?\n/g, '\r\n');
    if (this.pending) {
      this.pending.push(converted);
    } else {
      this.writer.fire(converted);
    }
  }

  dispose(): void {
    this.terminal.dispose();
    this.writer.dispose();
    this.closer.dispose();
  }
}

/** Runs documents and keeps the terminal. */
export class Runner implements vscode.Disposable {
  private terminal: RunTerminal | undefined;
  /** The running request of each document, by URI. */
  private readonly running = new Map<string, CancellationTokenSource>();
  private last: RunRecord | undefined;

  constructor(private readonly client: () => LanguageClient | undefined) {}

  /** The latest finished run. */
  lastRun(): RunRecord | undefined {
    return this.last;
  }

  /** `datascript.runStatement`: from a CodeLens (URI and statement), or at the cursor. */
  async runStatement(uri?: string, statement?: number): Promise<RunRecord | undefined> {
    // Trust and the server come first: without them there is no statement to find, and the
    // user should hear the real reason.
    const client = this.runnableClient();
    if (!client) {
      return undefined;
    }
    if (uri !== undefined && statement !== undefined) {
      return this.run(client, vscode.Uri.parse(uri), statement);
    }
    const editor = vscode.window.activeTextEditor;
    if (!editor || editor.document.languageId !== 'datascript') {
      void vscode.window.showInformationMessage('Open a DataScript file to run a statement.');
      return undefined;
    }
    const at = await this.statementAt(client, editor.document, editor.selection.active);
    if (at === undefined) {
      void vscode.window.showInformationMessage(
        'Put the cursor in a query statement to run it. `let` statements run with the queries that use them.',
      );
      return undefined;
    }
    return this.run(client, editor.document.uri, at);
  }

  /** `datascript.runFile`: from a CodeLens (URI), or the active editor. */
  async runFile(uri?: string): Promise<RunRecord | undefined> {
    const client = this.runnableClient();
    if (!client) {
      return undefined;
    }
    if (uri !== undefined) {
      return this.run(client, vscode.Uri.parse(uri));
    }
    const editor = vscode.window.activeTextEditor;
    if (!editor || editor.document.languageId !== 'datascript') {
      void vscode.window.showInformationMessage('Open a DataScript file to run it.');
      return undefined;
    }
    return this.run(client, editor.document.uri);
  }

  /** Cancels every run. */
  cancelAll(): void {
    for (const source of this.running.values()) {
      source.cancel();
    }
  }

  dispose(): void {
    this.cancelAll();
    this.terminal?.dispose();
    this.terminal = undefined;
  }

  /**
   * The client to run with, or `undefined` after telling the user why nothing can run: the
   * workspace is in Restricted Mode, or the server is not running.
   */
  private runnableClient(): LanguageClient | undefined {
    if (!vscode.workspace.isTrusted) {
      void vscode.window.showWarningMessage(
        'DataScript runs no queries in Restricted Mode. Trust the workspace to run it.',
      );
      return undefined;
    }
    const client = this.client();
    if (!client) {
      void vscode.window.showErrorMessage(
        'DataScript: the language server is not running, so nothing can run.',
      );
      return undefined;
    }
    return client;
  }

  /** The query statement whose CodeLens range holds `position`, 1-based. */
  private async statementAt(
    client: LanguageClient,
    document: vscode.TextDocument,
    position: vscode.Position,
  ): Promise<number | undefined> {
    const lenses = (await client.sendRequest('textDocument/codeLens', {
      textDocument: { uri: document.uri.toString() },
    })) as Array<{
      range: { start: { line: number }; end: { line: number } };
      command?: { command: string; arguments?: unknown[] };
    }> | null;
    for (const lens of lenses ?? []) {
      const { start, end } = lens.range;
      if (
        lens.command?.command === RUN_STATEMENT_COMMAND &&
        start.line <= position.line &&
        position.line <= end.line
      ) {
        return Number(lens.command.arguments?.[1]);
      }
    }
    return undefined;
  }

  private async run(
    client: LanguageClient,
    uri: vscode.Uri,
    statement?: number,
  ): Promise<RunRecord | undefined> {
    // The server runs documents it has open; opening is a no-op when the editor has it already.
    await vscode.workspace.openTextDocument(uri);
    const key = uri.toString();
    this.running.get(key)?.cancel();
    const source = new CancellationTokenSource();
    this.running.set(key, source);

    const terminal = this.ensureTerminal();
    terminal.terminal.show(true);
    let text = '';
    const write = (chunk: string) => {
      text += chunk;
      terminal.write(chunk);
    };
    const what = statement === undefined ? 'whole file' : `statement ${statement}`;
    write(`\x1b[1m▶ ${vscode.workspace.asRelativePath(uri)} (${what})\x1b[0m\n`);

    const params: RunParams = {
      textDocument: { uri: key },
      statement,
      maxRows: vscode.workspace.getConfiguration('datascript.run').get<number>('maxRows', 100),
      color: true,
    };
    const started = Date.now();
    const took = () => `${((Date.now() - started) / 1000).toFixed(2)} s`;
    let record: RunRecord;
    try {
      const result = (await client.sendRequest(RUN_METHOD, params, source.token)) as RunResult;
      write(result.output);
      write(
        result.failed
          ? `\x1b[2mFailed after ${took()}.\x1b[0m\n\n`
          : `\x1b[2mDone in ${took()}.\x1b[0m\n\n`,
      );
      record = {
        uri: key,
        statement,
        outcome: result.failed ? 'failed' : 'ok',
        output: result.output,
        terminalText: '',
      };
    } catch (error) {
      if (source.token.isCancellationRequested || isCancelled(error)) {
        write(`\x1b[2mCancelled after ${took()}.\x1b[0m\n\n`);
        record = { uri: key, statement, outcome: 'cancelled', output: '', terminalText: '' };
      } else {
        const reason = error instanceof Error ? error.message : String(error);
        write(`\x1b[31merror:\x1b[0m ${reason}\n\n`);
        record = { uri: key, statement, outcome: 'refused', output: reason, terminalText: '' };
      }
    } finally {
      if (this.running.get(key) === source) {
        this.running.delete(key);
      }
      source.dispose();
    }
    record.terminalText = text;
    this.last = record;
    return record;
  }

  private ensureTerminal(): RunTerminal {
    if (!this.terminal) {
      const created: RunTerminal = new RunTerminal(
        () => this.cancelAll(),
        () => {
          if (this.terminal === created) {
            this.terminal = undefined;
          }
          this.cancelAll();
        },
      );
      this.terminal = created;
    }
    return this.terminal;
  }
}

function isCancelled(error: unknown): boolean {
  return error instanceof ResponseError && error.code === LSPErrorCodes.RequestCancelled;
}
