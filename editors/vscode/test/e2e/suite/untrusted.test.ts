// An untrusted workspace (Restricted Mode): the server reports syntax errors only and reads no
// files, and the workspace cannot choose which program VS Code runs as the server.

import * as assert from 'node:assert/strict';
import * as vscode from 'vscode';
import type { DataScriptApi } from '../../../src/extension';
import { activate, codes, open, sleep, waitForCodes, write } from './helpers';

describe('in an untrusted workspace', () => {
  let api: DataScriptApi;

  before(async () => {
    assert.equal(vscode.workspace.isTrusted, false, 'the workspace opens in Restricted Mode');
    api = await activate();
  });

  it('ignores the workspace setting for the server path', () => {
    // The workspace's `.vscode/settings.json` names `./not-ds`; the server started anyway, from
    // the user setting.
    const inspected = vscode.workspace.getConfiguration('datascript.server').inspect('path');
    assert.equal(inspected?.workspaceValue, undefined);
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
});
