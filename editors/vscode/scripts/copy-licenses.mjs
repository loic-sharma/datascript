// The repository's license files, copied next to the manifest so `vsce package` includes them.
import { copyFileSync } from 'node:fs';

for (const name of ['LICENSE-MIT', 'LICENSE-APACHE']) {
  copyFileSync(new URL(`../../../${name}`, import.meta.url), new URL(`../${name}`, import.meta.url));
}
