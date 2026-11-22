// Checks the TextMate grammar against the real lexer.
//
// `tokens.json` holds the lexer's tokens for each fixture file (written by the Rust test
// crates/datascript-syntax/tests/textmate_tokens.rs, which also checks that the fixtures contain
// every token kind). Here VS Code's own tokenizer runs the grammar over the same files, and
// every character of every token must carry the scope its kind calls for. Then a few cases check
// rules the lexer does not know about, such as operator names after `|` (D-026).

import * as assert from 'node:assert/strict';
import * as fs from 'node:fs';
import * as path from 'node:path';
import * as oniguruma from 'vscode-oniguruma';
import * as textmate from 'vscode-textmate';

const root = path.resolve(__dirname, '../../..');
const fixtures = path.join(root, 'test/grammar');
const grammarPath = path.join(root, 'syntaxes/datascript.tmLanguage.json');

/** Scopes a character may have, by lexer token kind: one of these must prefix one of its scopes. */
const EXPECTED: Record<string, string[]> = {
  WHITESPACE: [],
  LINE_COMMENT: ['comment.line.double-slash'],
  BLOCK_COMMENT: ['comment.block'],
  PIPE: ['keyword.operator.pipe'],
  SEMICOLON: ['punctuation.terminator.statement'],
  COMMA: ['punctuation.separator.comma'],
  L_PAREN: ['punctuation.section.parens.begin'],
  R_PAREN: ['punctuation.section.parens.end'],
  L_BRACK: ['punctuation.section.brackets.begin'],
  R_BRACK: ['punctuation.section.brackets.end'],
  L_BRACE: ['punctuation.section.braces.begin'],
  R_BRACE: ['punctuation.section.braces.end'],
  DOT: ['punctuation.accessor'],
  DOT2: ['keyword.operator.range'],
  COLON: ['punctuation.separator.colon'],
  EQ: ['keyword.operator.assignment'],
  EQ2: ['keyword.operator.comparison'],
  NEQ: ['keyword.operator.comparison'],
  LT: ['keyword.operator.comparison'],
  LTEQ: ['keyword.operator.comparison'],
  GT: ['keyword.operator.comparison'],
  GTEQ: ['keyword.operator.comparison'],
  LTGT: ['invalid.illegal.operator'],
  EQ_TILDE: ['keyword.operator.comparison'],
  BANG_TILDE: ['keyword.operator.comparison'],
  PLUS: ['keyword.operator.arithmetic'],
  // Also the hyphen of a joined operator name such as `project-away`.
  MINUS: ['keyword.operator.arithmetic', 'keyword.control.operator'],
  STAR: ['keyword.operator.arithmetic'],
  SLASH: ['keyword.operator.arithmetic'],
  PERCENT: ['keyword.operator.arithmetic'],
  BANG: ['invalid.illegal.operator'],
  TILDE: ['keyword.operator.tilde'],
  AMP2: ['invalid.illegal.operator'],
  PIPE2: ['invalid.illegal.operator'],
  DOLLAR_LEFT: ['variable.language'],
  DOLLAR_RIGHT: ['variable.language'],
  NEGATED_OP: ['keyword.operator.word.negated'],
  IN_TILDE: ['keyword.operator.word'],
  LET_KW: ['storage.type.let'],
  TRUE_KW: ['constant.language.boolean'],
  FALSE_KW: ['constant.language.boolean'],
  NULL_KW: ['constant.language.null'],
  AND_KW: ['keyword.operator.logical'],
  OR_KW: ['keyword.operator.logical'],
  NOT_KW: ['keyword.operator.logical'],
  // Contextual keywords, operator names and function names are identifiers to the lexer.
  IDENT: [
    'variable.other.datascript',
    'entity.name.function',
    'keyword.control.operator',
    'keyword.other',
    'keyword.operator.word.datascript',
  ],
  QUOTED_IDENT: ['variable.other.quoted'],
  LONG: ['constant.numeric.integer'],
  REAL: ['constant.numeric.float'],
  TIMESPAN: ['constant.numeric.timespan', 'constant.other.timespan'],
  DATETIME: ['constant.other.datetime'],
  STRING: ['string.quoted'],
  ERROR: ['invalid.illegal'],
};

type Segment = [line: number, start: number, end: number, kind: string];

let grammar: textmate.IGrammar;

before(async () => {
  const wasm = fs.readFileSync(require.resolve('vscode-oniguruma/release/onig.wasm'));
  await oniguruma.loadWASM(wasm.buffer.slice(wasm.byteOffset, wasm.byteOffset + wasm.byteLength));
  const registry = new textmate.Registry({
    onigLib: Promise.resolve({
      createOnigScanner: (patterns: string[]) => new oniguruma.OnigScanner(patterns),
      createOnigString: (text: string) => new oniguruma.OnigString(text),
    }),
    loadGrammar: async (scopeName: string) =>
      scopeName === 'source.datascript'
        ? textmate.parseRawGrammar(fs.readFileSync(grammarPath, 'utf8'), grammarPath)
        : null,
  });
  const loaded = await registry.loadGrammar('source.datascript');
  assert.ok(loaded, 'the grammar loads');
  grammar = loaded;
});

/** The scopes of every UTF-16 unit of every line of `text`, without the root scope. */
function scopes(text: string): string[][][] {
  let stack = textmate.INITIAL;
  return text.split(/\r\n|\r|\n/).map((line) => {
    const result = grammar.tokenizeLine(line, stack);
    stack = result.ruleStack;
    const perChar: string[][] = [];
    for (const token of result.tokens) {
      for (let i = token.startIndex; i < Math.min(token.endIndex, line.length); i++) {
        perChar[i] = token.scopes.slice(1);
      }
    }
    return perChar;
  });
}

function describeSpot(text: string, line: number, start: number, end: number): string {
  const source = text.split(/\r\n|\r|\n/)[line];
  return `line ${line + 1}, \`${source.slice(start, end)}\` in \`${source}\``;
}

describe('the grammar agrees with the lexer', () => {
  const tokens: Record<string, Segment[]> = JSON.parse(
    fs.readFileSync(path.join(fixtures, 'tokens.json'), 'utf8'),
  );

  it('has an expected scope for every token kind in the fixtures', () => {
    const kinds = new Set(Object.values(tokens).flatMap((segs) => segs.map((s) => s[3])));
    const unknown = [...kinds].filter((kind) => !(kind in EXPECTED));
    assert.deepEqual(unknown, [], 'add these kinds to EXPECTED');
  });

  for (const [file, segments] of Object.entries(tokens)) {
    it(`scopes every token of ${file}`, () => {
      const text = fs.readFileSync(path.join(fixtures, file), 'utf8');
      const lines = scopes(text);
      const failures: string[] = [];
      for (const [line, start, end, kind] of segments) {
        const allowed = EXPECTED[kind];
        for (let i = start; i < end; i++) {
          const actual = lines[line]?.[i] ?? [];
          const ok =
            allowed.length === 0
              ? actual.length === 0
              : actual.some((scope) => allowed.some((prefix) => scope.startsWith(prefix)));
          if (!ok) {
            failures.push(
              `${kind} at ${describeSpot(text, line, start, end)}: column ${i + 1} has ` +
                `[${actual.join(', ')}], expected ${allowed.length ? allowed.join(' or ') : 'no scope'}`,
            );
            break;
          }
        }
      }
      assert.deepEqual(failures, []);
    });
  }
});

/** The innermost scope of each word of `line` that `words` names, in order. */
function scopesOf(text: string, line: number, ...words: string[]): string[] {
  const source = text.split('\n')[line];
  const perChar = scopes(text)[line];
  let from = 0;
  return words.map((word) => {
    const at = source.indexOf(word, from);
    assert.ok(at >= 0, `\`${word}\` is on line ${line + 1}`);
    from = at + word.length;
    const all = perChar[at] ?? [];
    for (let i = at; i < at + word.length; i++) {
      assert.deepEqual(perChar[i] ?? [], all, `\`${word}\` has one scope throughout`);
    }
    return all[all.length - 1] ?? '';
  });
}

describe('operator names', () => {
  const OP = 'keyword.control.operator.datascript';
  const VAR = 'variable.other.datascript';
  const MINUS = 'keyword.operator.arithmetic.datascript';

  it('are the identifier right after `|`', () => {
    assert.deepEqual(scopesOf('from t | where where > 1', 0, 'where', 'where'), [OP, VAR]);
    assert.deepEqual(scopesOf('from t | count', 0, 'count'), [OP]);
    assert.deepEqual(scopesOf('from t | summarize count() by k', 0, 'summarize'), [OP]);
  });

  it('may follow `|` after trivia on later lines', () => {
    const text = 'from t\n|\n  // note\n  /* more */\n  where x';
    assert.deepEqual(scopesOf(text, 4, 'where', 'x'), [OP, VAR]);
    assert.deepEqual(scopesOf('from t | /* c */ take 1', 0, 'take'), [OP]);
  });

  it('join a known hyphenated name only when its tokens are adjacent (D-026)', () => {
    for (const name of [
      'project-away',
      'project-rename',
      'project-reorder',
      'mv-expand',
      'make-series',
    ]) {
      assert.deepEqual(scopesOf(`from t | ${name} x`, 0, name, 'x'), [OP, VAR], name);
    }
    assert.deepEqual(scopesOf('from t | project - away x', 0, 'project', '-', 'away'), [
      OP,
      MINUS,
      VAR,
    ]);
    assert.deepEqual(scopesOf('from t | project -away x', 0, 'project', '-', 'away'), [
      OP,
      MINUS,
      VAR,
    ]);
    // Unknown names are not joined, as in the parser.
    assert.deepEqual(scopesOf('from t | project-foo x', 0, 'project', '-', 'foo'), [
      OP,
      MINUS,
      VAR,
    ]);
  });

  it('are not read elsewhere', () => {
    assert.deepEqual(scopesOf('from t | where a - b > 0', 0, 'a', '-', 'b'), [VAR, MINUS, VAR]);
    assert.deepEqual(scopesOf('from t | extend x = project-away', 0, 'project', '-', 'away'), [
      VAR,
      MINUS,
      VAR,
    ]);
    assert.deepEqual(scopesOf('from t | where count > 1', 0, 'count'), [VAR]);
  });

  it('are not reserved words or non-ASCII words', () => {
    assert.deepEqual(scopesOf('from t | not x', 0, 'not'), ['keyword.operator.logical.datascript']);
    assert.deepEqual(scopesOf('from t | naïve', 0, 'naïve'), ['invalid.illegal.name.datascript']);
    assert.deepEqual(scopesOf('from t | 5', 0, '5'), ['constant.numeric.integer.datascript']);
  });

  it('do not follow `||`, which is lexed for a better error', () => {
    assert.deepEqual(scopesOf('from t || where x', 0, '||', 'where'), [
      'invalid.illegal.operator.datascript',
      VAR,
    ]);
  });
});

describe('`from`', () => {
  const FROM = 'keyword.other.from.datascript';
  const VAR = 'variable.other.datascript';

  it('is a keyword at the head of a pipeline', () => {
    assert.deepEqual(scopesOf("from 'a.csv'", 0, 'from'), [FROM]);
    assert.deepEqual(scopesOf('let x = (from t | count)', 0, 'from'), [FROM]);
    assert.deepEqual(scopesOf('from(from t)', 0, 'from', 'from'), [FROM, FROM]);
    assert.deepEqual(scopesOf('from from | take 1', 0, 'from', 'from'), [FROM, VAR]);
  });

  it('is a name where no source follows (D-036)', () => {
    assert.deepEqual(scopesOf('from t | where from > 3', 0, 'from', 'from'), [FROM, VAR]);
    assert.deepEqual(scopesOf('let y = from - 1', 0, 'from'), [VAR]);
    assert.deepEqual(scopesOf('let y = from not', 0, 'from'), [VAR]);
    assert.deepEqual(scopesOf('let y = from and x', 0, 'from'), [VAR]);
    assert.deepEqual(scopesOf('let from = 1', 0, 'from'), [VAR]);
  });
});
