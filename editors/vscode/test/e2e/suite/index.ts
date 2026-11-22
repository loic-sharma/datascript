// The entry point VS Code calls (`--extensionTestsPath`): runs the suite for the current mode.

import * as path from 'node:path';
import Mocha from 'mocha';

export function run(): Promise<void> {
  const mode = process.env.DS_E2E_MODE;
  if (mode !== 'trusted' && mode !== 'untrusted') {
    return Promise.reject(new Error(`DS_E2E_MODE must be trusted or untrusted, not ${mode}`));
  }
  const mocha = new Mocha({ ui: 'bdd', color: true, timeout: 60_000 });
  mocha.addFile(path.join(__dirname, `${mode}.test.js`));
  return new Promise((resolve, reject) => {
    mocha.run((failures) =>
      failures > 0 ? reject(new Error(`${failures} test(s) failed`)) : resolve(),
    );
  });
}
