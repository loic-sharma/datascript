// A trusted workspace: the server runs the full check, reading the data files.

import * as assert from 'node:assert/strict';
import * as vscode from 'vscode';
import type { DataScriptApi } from '../../../src/extension';
import type { RunRecord } from '../../../src/run';
import { activate, codeLenses, open, sleep, terminalNamed, waitForCodes, write } from './helpers';

describe('in a trusted workspace', () => {
  let api: DataScriptApi;

  before(async () => {
    assert.equal(vscode.workspace.isTrusted, true);
    api = await activate();
  });

  it('starts the server with the full check', () => {
    assert.deepEqual(api.initializationOptions(), { syntaxOnly: false });
  });

  it('highlights and opens `.ds` files as DataScript', async () => {
    const document = await open('empty.ds', '');
    assert.equal(document.languageId, 'datascript');
  });

  it('shows diagnostics from the server, schema errors included', async () => {
    const document = await open('query.ds', "from 'orders.csv'\n| summarize total = sum(amout)\n");
    const [diagnostic] = await waitForCodes(document.uri, ['DS1001']);
    assert.match(diagnostic.message, /^unknown column `amout`/);
    assert.equal(diagnostic.severity, vscode.DiagnosticSeverity.Error);
    assert.equal(diagnostic.source, 'datascript');
    assert.deepEqual(
      [diagnostic.range.start.line, diagnostic.range.start.character, diagnostic.range.end.character],
      [1, 24, 29],
    );
    const code = diagnostic.code as { value: string; target: vscode.Uri };
    assert.match(code.target.toString(), /DIAGNOSTICS\.md/);

    // Fixing it as the user would clears it.
    const editor = await vscode.window.showTextDocument(document);
    await editor.edit((edit) => edit.replace(new vscode.Range(1, 24, 1, 29), 'amount'));
    await waitForCodes(document.uri, []);
  });

  it('shows syntax errors and warnings', async () => {
    const document = await open(
      'mixed.ds',
      "from 'orders.csv' | where amount = 10 | sort by orderId | take 1\n",
    );
    await waitForCodes(document.uri, ['DS0110', 'DS2103']);
  });

  it('checks again when a data file changes on disk', async () => {
    // The server registers a file watcher with the client (dynamic registration, D-052).
    const document = await open('watch.ds', "from 'regions.csv' | project region\n");
    await waitForCodes(document.uri, ['DS3001']);
    write('regions.csv', 'id,region\n1,eu\n');
    await waitForCodes(document.uri, []);
    write('regions.csv', 'id,area\n1,eu\n');
    await waitForCodes(document.uri, ['DS1001']);
  });

  it('restarts the server on request', async () => {
    const before = api.client();
    await vscode.commands.executeCommand('datascript.restartServer');
    await api.ready();
    assert.ok(api.client());
    assert.notEqual(api.client(), before);
    const document = await open('after-restart.ds', "from 'orders.csv' | project amout\n");
    await waitForCodes(document.uri, ['DS1001']);
  });

  describe('Run', () => {
    const program =
      "let big = from 'orders.csv' | where amount > 15;\n" +
      "from 'orders.csv' | count;\n" +
      'from big | project orderId\n';

    it('shows a Run CodeLens over each query statement', async () => {
      const document = await open('lenses.ds', program);
      await waitForCodes(document.uri, []);
      const lenses = await codeLenses(document.uri);
      assert.deepEqual(
        lenses.map((lens) => [lens.range.start.line, lens.command?.title, lens.command?.arguments]),
        [
          [1, '▶ Run', [document.uri.toString(), 1]],
          [1, '▶ Run file', [document.uri.toString()]],
          [2, '▶ Run', [document.uri.toString(), 2]],
        ],
      );
    });

    it('runs a statement from its CodeLens, with the lets it needs', async () => {
      const document = await open('statement.ds', program);
      const [, , second] = await codeLenses(document.uri);
      const command = second.command!;
      const record = (await vscode.commands.executeCommand(
        command.command,
        ...(command.arguments ?? []),
      )) as RunRecord;
      assert.equal(record.outcome, 'ok');
      assert.equal(record.output, 'orderId\nlong\n-------\n      2\n');
      assert.match(record.terminalText, /^\x1b\[1m▶ statement\.ds \(statement 2\)/);
      assert.match(record.terminalText, /Done in \d+\.\d\d s\./);
      assert.deepEqual(api.lastRun(), record);
    });

    it('runs the whole file, and the statement at the cursor', async () => {
      const document = await open('file.ds', program);
      const record = (await vscode.commands.executeCommand('datascript.runFile')) as RunRecord;
      assert.equal(record.outcome, 'ok');
      assert.match(record.output, /^== query 1 \(line 2\) ==\n/);
      assert.match(record.output, /== query 2 \(line 3\) ==\n/);

      const editor = await vscode.window.showTextDocument(document);
      editor.selection = new vscode.Selection(1, 5, 1, 5);
      const atCursor = (await vscode.commands.executeCommand(
        'datascript.runStatement',
      )) as RunRecord;
      assert.equal(atCursor.statement, 1);
      assert.equal(atCursor.output, 'Count\nlong\n-----\n    2\n');
    });

    it('runs unsaved edits', async () => {
      const document = await open('unsaved.ds', "from 'orders.csv' | count\n");
      const editor = await vscode.window.showTextDocument(document);
      await editor.edit((edit) =>
        edit.replace(new vscode.Range(0, 20, 0, 25), 'summarize total = sum(amount)'),
      );
      assert.equal(document.isDirty, true);
      const record = (await vscode.commands.executeCommand('datascript.runFile')) as RunRecord;
      assert.equal(record.output, 'total\nreal\n-----\n 30.5\n');
    });

    it('shows a runtime error in the terminal and as a diagnostic', async () => {
      write('big.csv', 'x\n1\n10000\n');
      const document = await open(
        'overflow.ds',
        "from 'big.csv' | extend y = x * 1000000000000000\n",
      );
      await waitForCodes(document.uri, []);
      const record = (await vscode.commands.executeCommand('datascript.runFile')) as RunRecord;
      assert.equal(record.outcome, 'failed');
      assert.match(record.output, /error\[DS4003\]/);
      assert.match(record.terminalText, /Failed after/);
      await waitForCodes(document.uri, ['DS4003']);

      // The next check after an edit clears it.
      const editor = await vscode.window.showTextDocument(document);
      await editor.edit((edit) => edit.replace(new vscode.Range(0, 32, 0, 48), '2'));
      await waitForCodes(document.uri, []);
    });

    describe('cancellation', () => {
      // A query slow enough to cancel: a few million rows, read in full.
      const slow = "from 'huge.csv' | summarize n = count() by k = x % 1000 | sort by k\n";

      before(() => {
        const rows = Array.from({ length: 3_000_000 }, (_, i) => i).join('\n');
        write('huge.csv', `x\n${rows}\n`);
      });

      it('stops a run when the user presses Ctrl+C in the terminal', async () => {
        const document = await open('cancel.ds', slow);
        const running = vscode.commands.executeCommand('datascript.runFile', document.uri.toString());
        const terminal = await terminalNamed('DataScript');
        await sleep(200);
        terminal.sendText('\x03', false);
        const record = (await running) as RunRecord;
        assert.equal(record.outcome, 'cancelled', record.terminalText);
        assert.match(record.terminalText, /Cancelled after/);
      });

      it('cancels the previous run of a document when it runs again', async () => {
        const document = await open('again.ds', slow);
        const first = vscode.commands.executeCommand('datascript.runFile', document.uri.toString());
        await sleep(200);
        const second = vscode.commands.executeCommand('datascript.runFile', document.uri.toString());
        const firstRecord = (await first) as RunRecord;
        assert.equal(firstRecord.outcome, 'cancelled', firstRecord.terminalText);
        assert.equal(((await second) as RunRecord).outcome, 'ok');
      });
    });
  });
});
