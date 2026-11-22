// A trusted workspace: the server runs the full check, reading the data files.

import * as assert from 'node:assert/strict';
import * as vscode from 'vscode';
import type { DataScriptApi } from '../../../src/extension';
import { activate, open, waitForCodes, write } from './helpers';

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
});
