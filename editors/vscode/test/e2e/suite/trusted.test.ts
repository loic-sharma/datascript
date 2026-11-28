// A trusted workspace: the server runs the full check, reading the data files.

import * as assert from 'node:assert/strict';
import * as vscode from 'vscode';
import type { DataScriptApi } from '../../../src/extension';
import type { RunRecord } from '../../../src/run';
import {
  activate,
  captureMessages,
  codeLenses,
  open,
  sleep,
  terminalNamed,
  updateServerSetting,
  waitForCodes,
  write,
} from './helpers';

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

  it('shows diagnostics for an untitled document, reading files in the workspace folder', async () => {
    const document = await vscode.workspace.openTextDocument({
      language: 'datascript',
      content: "from 'orders.csv' | project amout\n",
    });
    await vscode.window.showTextDocument(document);
    assert.equal(document.uri.scheme, 'untitled');
    try {
      const [diagnostic] = await waitForCodes(document.uri, ['DS1001']);
      assert.match(diagnostic.message, /^unknown column `amout`/);
    } finally {
      await vscode.commands.executeCommand('workbench.action.revertAndCloseActiveEditor');
    }
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

  it('restarts the server when a datascript.server setting changes', async () => {
    const before = api.client();
    assert.ok(before);
    try {
      await updateServerSetting(api, 'extraArgs', ['--color', 'never']);
      assert.ok(api.client());
      assert.notEqual(api.client(), before);
      const document = await open('after-config.ds', "from 'orders.csv' | project amout\n");
      await waitForCodes(document.uri, ['DS1001']);
    } finally {
      await updateServerSetting(api, 'extraArgs', undefined);
    }
    assert.ok(api.client(), 'server restarts after restoring datascript.server.extraArgs');
  });

  it('reports start failure for an invalid server path while highlighting still works', async () => {
    try {
      const errors = await captureMessages('showErrorMessage', async () => {
        await updateServerSetting(api, 'path', './no-such-ds-binary');
      });
      assert.equal(api.client(), undefined);
      assert.equal(api.initializationOptions(), undefined);
      const report = errors.find((e) => e.message.startsWith('DataScript: cannot find'));
      assert.ok(
        report,
        `expected reportStartFailure notification, got: ${JSON.stringify(errors)}`,
      );
      assert.match(report.message, /no-such-ds-binary/);
      assert.match(report.message, /Highlighting works without it\./);
      assert.deepEqual(report.items, ['Open Settings', 'Show Log']);

      // Highlighting (TextMate grammar) does not depend on the language server.
      const document = await open('no-server.ds', "from 'orders.csv' | where amount > 10\n");
      assert.equal(document.languageId, 'datascript');
      const tokens = await vscode.commands.executeCommand<Array<{ c: string; t: string }>>(
        '_workbench.captureSyntaxTokens',
        document.uri,
      );
      assert.ok(tokens && tokens.length > 0, 'TextMate tokens captured without a running server');
      const scopeOf = (text: string) => tokens.find((token) => token.c === text)?.t ?? '';
      assert.match(scopeOf('from'), /\bkeyword\./);
      assert.match(scopeOf('orders.csv'), /\bstring\.quoted\./);
      assert.match(scopeOf('where'), /\bkeyword\.control\.operator\./);
      assert.match(scopeOf('10'), /\bconstant\.numeric\./);
    } finally {
      await updateServerSetting(api, 'path', undefined);
    }
    assert.ok(api.client(), 'server restarts after restoring datascript.server.path');
  });

  it('says the server is stopped when running the statement at the cursor without it', async () => {
    const document = await open('stopped.ds', "from 'orders.csv' | count\n");
    await waitForCodes(document.uri, []);
    try {
      await captureMessages('showErrorMessage', async () => {
        await updateServerSetting(api, 'path', './no-such-ds-binary');
      });
      assert.equal(api.client(), undefined);
      const editor = await vscode.window.showTextDocument(document);
      editor.selection = new vscode.Selection(0, 5, 0, 5);
      const previous = api.lastRun();
      // The cursor is in a query, but there is no server to run it (or to find it with): the
      // user hears that, not "Put the cursor in a query statement".
      const infos = await captureMessages('showInformationMessage', async () => {
        const errors = await captureMessages('showErrorMessage', async () => {
          assert.equal(await vscode.commands.executeCommand('datascript.runStatement'), undefined);
          assert.equal(await vscode.commands.executeCommand('datascript.runFile'), undefined);
        });
        const stopped = 'DataScript: the language server is not running, so nothing can run.';
        assert.deepEqual(
          errors.map((m) => m.message),
          [stopped, stopped],
        );
      });
      assert.deepEqual(infos, []);
      assert.equal(api.lastRun(), previous);
    } finally {
      await updateServerSetting(api, 'path', undefined);
    }
    assert.ok(api.client(), 'server restarts after restoring datascript.server.path');
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

    it('runs nothing on a let or blank line, and runs a multi-line query from line 2 or later', async () => {
      const multiline =
        "let big = from 'orders.csv' | where amount > 15;\n" +
        '\n' +
        "from 'orders.csv' | count;\n" +
        '\n' +
        'from big\n' +
        '| project orderId\n' +
        '| sort by orderId asc\n';
      const document = await open('cursor.ds', multiline);
      await waitForCodes(document.uri, []);
      const editor = await vscode.window.showTextDocument(document);
      const previous = api.lastRun();
      const expectedInfo =
        'Put the cursor in a query statement to run it. `let` statements run with the queries that use them.';

      await captureMessages('showInformationMessage', async (infos) => {
        // Cursor on a `let` statement runs nothing and shows the info message.
        editor.selection = new vscode.Selection(0, 4, 0, 4);
        assert.equal(await vscode.commands.executeCommand('datascript.runStatement'), undefined);
        assert.equal(api.lastRun(), previous);

        // Cursor on a blank line (after `let` or between queries) runs nothing and shows the info message.
        editor.selection = new vscode.Selection(1, 0, 1, 0);
        assert.equal(await vscode.commands.executeCommand('datascript.runStatement'), undefined);
        assert.equal(api.lastRun(), previous);

        editor.selection = new vscode.Selection(3, 0, 3, 0);
        assert.equal(await vscode.commands.executeCommand('datascript.runStatement'), undefined);
        assert.equal(api.lastRun(), previous);

        assert.deepEqual(
          infos.map((m) => m.message),
          [expectedInfo, expectedInfo, expectedInfo],
        );

        // Cursor on line 2 of the multi-line query runs that query with no info message.
        infos.length = 0;
        editor.selection = new vscode.Selection(5, 5, 5, 5);
        const line2 = (await vscode.commands.executeCommand(
          'datascript.runStatement',
        )) as RunRecord;
        assert.equal(line2.outcome, 'ok');
        assert.equal(line2.statement, 2);
        assert.equal(line2.output, 'orderId\nlong\n-------\n      2\n');
        assert.deepEqual(infos, []);

        // Cursor on line 3 of the multi-line query runs the same query.
        editor.selection = new vscode.Selection(6, 5, 6, 5);
        const line3 = (await vscode.commands.executeCommand(
          'datascript.runStatement',
        )) as RunRecord;
        assert.equal(line3.outcome, 'ok');
        assert.equal(line3.statement, 2);
        assert.equal(line3.output, 'orderId\nlong\n-------\n      2\n');
        assert.deepEqual(infos, []);
      });
    });

    it('cuts terminal output according to datascript.run.maxRows', async () => {
      write('many.csv', 'n\n1\n2\n3\n4\n5\n');
      const document = await open('max-rows.ds', "from 'many.csv' | sort by n asc\n");
      await waitForCodes(document.uri, []);
      const config = vscode.workspace.getConfiguration('datascript.run');
      await config.update('maxRows', 2, vscode.ConfigurationTarget.Workspace);
      try {
        const record = (await vscode.commands.executeCommand('datascript.runFile')) as RunRecord;
        assert.equal(record.outcome, 'ok');
        assert.equal(
          record.output,
          'n\nlong\n----\n   1\n   2\n… 3 more rows (show them with --max-rows 5)\n',
        );
        assert.match(record.terminalText, /… 3 more rows \(show them with --max-rows 5\)/);
      } finally {
        await config.update('maxRows', undefined, vscode.ConfigurationTarget.Workspace);
      }
      const full = (await vscode.commands.executeCommand('datascript.runFile')) as RunRecord;
      assert.equal(full.outcome, 'ok');
      assert.doesNotMatch(full.output, /more rows/);
      assert.match(full.output, /   5\n$/);
    });

    it('shows a compile error (DS1001) and chart info (DS5001) in the terminal', async () => {
      const bad = await open('compile-error.ds', "from 'orders.csv' | project amout\n");
      await waitForCodes(bad.uri, ['DS1001']);
      const failed = (await vscode.commands.executeCommand('datascript.runFile')) as RunRecord;
      assert.equal(failed.outcome, 'failed');
      assert.match(failed.output, /error\[DS1001\]/);
      assert.match(failed.terminalText, /error\[DS1001\]/);
      assert.match(failed.terminalText, /Failed after \d+\.\d\d s\./);

      const chart = await open(
        'chart.ds',
        "from 'orders.csv' | project customerId, amount | sort by customerId asc | render barchart\n",
      );
      await waitForCodes(chart.uri, []);
      const rendered = (await vscode.commands.executeCommand('datascript.runFile')) as RunRecord;
      assert.equal(rendered.outcome, 'ok');
      assert.match(rendered.output, /info\[DS5001\]/);
      assert.match(rendered.terminalText, /info\[DS5001\]/);
      assert.match(rendered.output, /c-1\s+\|\s+10\.5/);
      assert.match(rendered.terminalText, /Done in \d+\.\d\d s\./);
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

      it('cancels a run when the user closes the DataScript terminal', async () => {
        const document = await open('close-terminal.ds', slow);
        const running = vscode.commands.executeCommand('datascript.runFile', document.uri.toString());
        const terminal = await terminalNamed('DataScript');
        await sleep(200);
        terminal.show(false);
        await vscode.commands.executeCommand('workbench.action.terminal.kill');
        const record = (await running) as RunRecord;
        assert.equal(record.outcome, 'cancelled', record.terminalText);
        assert.match(record.terminalText, /Cancelled after/);

        // A later run opens a fresh DataScript terminal and succeeds.
        const followUp = await open('after-close.ds', "from 'orders.csv' | count\n");
        const ok = (await vscode.commands.executeCommand(
          'datascript.runFile',
          followUp.uri.toString(),
        )) as RunRecord;
        assert.equal(ok.outcome, 'ok');
        const reopened = await terminalNamed('DataScript');
        assert.notEqual(reopened, terminal);
      });
    });
  });
});
