// An untrusted workspace (Restricted Mode): the server reports syntax errors only and reads no
// files, and the workspace cannot choose which program VS Code runs as the server.

import * as assert from 'node:assert/strict';
import * as vscode from 'vscode';
import type { DataScriptApi } from '../../../src/extension';
import {
  activate,
  captureMessages,
  codeLenses,
  codes,
  open,
  sleep,
  waitForCodes,
  write,
} from './helpers';

const RESTRICTED = 'DataScript runs no queries in Restricted Mode. Trust the workspace to run it.';

describe('in an untrusted workspace', () => {
  let api: DataScriptApi;

  before(async () => {
    assert.equal(vscode.workspace.isTrusted, false, 'the workspace opens in Restricted Mode');
    api = await activate();
  });

  it('ignores the workspace settings for the server path and arguments', () => {
    // The workspace's `.vscode/settings.json` names `./not-ds` with the argument `x`; the server
    // started anyway, from the user setting and without the argument (`ds x lsp` would fail).
    const config = vscode.workspace.getConfiguration('datascript.server');
    assert.equal(config.inspect('path')?.workspaceValue, undefined);
    assert.equal(config.inspect('extraArgs')?.workspaceValue, undefined);
    assert.deepEqual(config.get('extraArgs'), []);
    assert.ok(api.client());
  });

  it('starts the server syntax-only', () => {
    assert.deepEqual(api.initializationOptions(), { syntaxOnly: true });
  });

  it('shows syntax errors but reads no schemas', async () => {
    // A syntax error and an unknown column: finding the second means reading orders.csv.
    const document = await open(
      'query.ds',
      "from 'orders.csv' | where amount = 1 | project amout | sort by orderId\n",
    );
    await waitForCodes(document.uri, ['DS0110']);
    // No schema check follows later either.
    await sleep(1_500);
    assert.deepEqual(codes(document.uri), ['DS0110']);
  });

  it('keeps checking syntax as the user types', async () => {
    const document = await open('typing.ds', "from 'orders.csv' | take 1\n");
    await waitForCodes(document.uri, []);
    const editor = await vscode.window.showTextDocument(document);
    await editor.edit((edit) => edit.insert(new vscode.Position(0, 26), ' # x'));
    await waitForCodes(document.uri, ['DS0001']);
  });

  it('ignores changed data files', async () => {
    const document = await open('watch.ds', "from 'regions.csv' | project region\n");
    await waitForCodes(document.uri, []);
    write('regions.csv', 'id,area\n1,eu\n');
    await sleep(1_500);
    assert.deepEqual(codes(document.uri), []);
  });

  it('shows no Run CodeLens and runs nothing, saying why', async () => {
    const document = await open('run.ds', "from 'orders.csv' | count;\nfrom 'orders.csv' | take 1\n");
    await waitForCodes(document.uri, []);
    assert.deepEqual(await codeLenses(document.uri), []);
    const editor = await vscode.window.showTextDocument(document);
    editor.selection = new vscode.Selection(0, 5, 0, 5);
    const infos = await captureMessages('showInformationMessage', async () => {
      const warnings = await captureMessages('showWarningMessage', async () => {
        assert.equal(await vscode.commands.executeCommand('datascript.runFile'), undefined);
        assert.equal(
          await vscode.commands.executeCommand('datascript.runStatement', document.uri.toString(), 1),
          undefined,
        );
        // At the cursor, the reason is Restricted Mode, not a missing statement: the syntax-only
        // server has no CodeLenses to find one with.
        assert.equal(await vscode.commands.executeCommand('datascript.runStatement'), undefined);
      });
      assert.deepEqual(
        warnings.map((m) => m.message),
        [RESTRICTED, RESTRICTED, RESTRICTED],
      );
    });
    assert.deepEqual(infos, []);
    assert.equal(api.lastRun(), undefined);
  });
});
