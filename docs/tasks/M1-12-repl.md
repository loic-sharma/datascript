# M1-12 — REPL

**Status:** done — `ds` with no command starts a reedline REPL (lets kept as text across inputs, `<repl:N>` snippets, redefinition, `.schema`/`.tables`/`.format`/`.help`/`.quit`, highlighting, completion, history, Ctrl-C cancels a query); piped stdin runs the same loop for scripts and tests.
**Depends on:** M1-11
**Crates:** `datascript-cli`

Extends the CLI from M1-11. The rules below were settled before implementation (D-048);
implement them, don't re-decide them.

## Goal
`ds` with no arguments starts an interactive session.

## Scope

### Starting the REPL
- `ds` with no subcommand starts the REPL. It accepts the global flags that make sense without a
  script: `--table name=path` (repeatable, same handling as `ds run`), `--color auto|always|never`
  and `--max-rows <n>` (default 100, 0 = all).
- Relative paths resolve against the current working directory (LANGUAGE.md §3.1, the table of
  base directories). The REPL is not sandboxed (D-016).
- **When stdin is not a terminal**, `ds` reads inputs from stdin with the same submit rule, but with
  no line editor, prompt, highlighting or history, and with the plain diagnostics renderer. It exits
  1 if any input had an error, otherwise 0. This mode exists for scripting and for end-to-end tests;
  `ds run -` is still the way to run a script from stdin.

### Entering input
- Line editor: `reedline`, with a `Validator` deciding whether Enter submits or adds a line.
- **Enter submits** when the input is complete *and* either ends with `;` or the current line is
  blank. Complete means, judged with the lexer (not by counting characters): brackets balanced, no
  unterminated string or block comment. `;`, brackets and `|` inside strings and comments don't
  count. So `from t | take 5` + Enter waits for another line (the next line may start with `| where`,
  the usual style); `from t | take 5;` + Enter runs it. A blank line submits incomplete input too,
  so the user gets the parser's diagnostic instead of being stuck.
- Enter on empty input does nothing. A **meta command** (the input starts with `.`) runs on Enter.
- The prompt is `ds> `, the continuation prompt `..> `.

### Lets across inputs (D-048)
- The REPL keeps the text of every saved `let` statement, in order. Each submission is checked and
  run as one program: the saved let text followed by the new input, built through a small
  `ReplProgram` type that also maps every span in the combined text back to the input it came from.
  No compiler API changes.
- **Each input is its own snippet** for diagnostics: input N is labelled `<repl:N>` (N counts
  submitted program inputs from 1, meta commands don't count) and its lines are numbered from 1.
  Register each saved input as its own `SourceFile` / `FileId` and rewrite each diagnostic's spans
  (primary, labels, and fix `TextEdit`s) to that file and local offset. An error in the new input
  renders against what the user just typed; an error inside a saved let (its file was deleted, say)
  renders against that let's original snippet.
- **Saving**: once the input checks without errors, its lets are saved, before any query runs. An
  input with any check error saves nothing. A runtime error or Ctrl-C in a query does not undo the
  saved lets.
- **Redefining a let is allowed in the REPL** (≠ the `DS1003` rule for programs). Entering
  `let x = …` when `x` is saved removes the old `x` *and every saved let that uses it, directly or
  transitively*, and prints one info line naming the removed lets so the user can enter them again.
  Use the binder's references from the HIR to find users; if they are not exposed, treating every
  identifier token that matches a saved let name as a use is acceptable (it can only remove too
  much, never too little). Two `let x` in the *same* input are still `DS1003`.
- Saved scalar lets are evaluated again in every submission, so `let cutoff = ago(1d)` follows the
  clock; within one submission `now()` is fixed (D-015).
- Call `Session::invalidate_schemas` before every submission and every `.schema`, so a file created
  or edited after an error is read again (D-043). There is no `.reload` command.

### Output
- Results print exactly as `ds run` prints them (reuse `output.rs`): `table` by default, cut at
  `--max-rows` with the "… N more rows" footer, `== query <n> (line <l>) ==` headers when one input
  has several queries. `render` behaves as it does in `ds run` (M1-10 decides the info line).
- Diagnostics go to stderr with the M1-11 renderer rules (terminal renderer on a TTY, `--color`,
  `NO_COLOR`).

### Meta commands
| Command | Does |
|---|---|
| `.help` | Lists the meta commands, the submit rule, and the keys (Ctrl-C, Ctrl-D, Tab). |
| `.schema <source>` | Prints one row per column (`column`, `type`) as a table. `<source>` is anything that can follow `from`: a quoted path, a host table name, or a tabular let. Implement it by checking `from <source>` as a hidden program, mapping spans back onto the argument, so source errors are the usual diagnostics (`DS3001` with did-you-mean, …). |
| `.tables` | Lists host tables (name, path) and saved lets (name, scalar or tabular). |
| `.format table\|csv\|json` | Sets the output format for the rest of the session. With no argument, prints the current one. `csv` with several queries in one input prints each result under its header line (unlike `ds run`, which refuses: an interactive user can read them). |
| `.quit`, `.exit` | Exit 0. |

- **Meta command mistakes** (unknown command, missing or bad argument) follow D-046: a plain
  one-line message with a tip, not a `Diagnostic` with a code, because there is no program location.
  An unknown command gets did-you-mean over the command names (`.shema` → `.schema`), using the
  same helper the compiler uses for names (`datascript_diagnostics::suggest::did_you_mean`).
- **No diagnostic codes are reserved for this task.** Program errors reuse existing codes. If a new
  code turns out to be necessary, stop and reserve a block in DIAGNOSTICS.md first (D-018).

### Keys
- **Ctrl-C while a query runs** cancels it (drop the run future, as M1-11 does, but return to the
  prompt instead of exiting), prints `query cancelled`, and keeps the session and saved lets. Use a
  `tokio::signal::ctrl_c` race per submission, not the process-exit handler from M1-11.
- **Ctrl-C at the prompt** discards the current input, including unfinished multi-line input. On an
  empty prompt it prints a hint: `press Ctrl-D or type .quit to exit`.
- **Ctrl-D** on an empty prompt exits 0. With text in the buffer it behaves as reedline's default.

### History
- Persistent history at `dirs::data_dir()/datascript/history` (`~/.local/share/…` on Linux,
  `~/Library/Application Support/…` on macOS), 1000 entries. A multi-line input is one entry; meta
  commands are stored too.
- If the file cannot be created or read, print one warning line and continue with in-memory history.

### Highlighting and completion
- Syntax highlighting from the lexer's token kinds: keywords and operators, strings, numbers,
  comments, and lexer errors. Off when color is off (`--color never`, `NO_COLOR`).
- Tab completes operator names (`datascript_syntax::OPERATOR_NAMES`), function names (from the
  function catalog), saved let names and host table names, and meta command names when the input
  starts with `.`. Column completion is M3-07 (it uses the compiler's `complete` from M3-02).

### New dependencies
`reedline` and `dirs`, added to the workspace `Cargo.toml` and used only by `datascript-cli`.
`datascript-cli` also gains a direct dependency on `datascript-syntax` (lexer for the validator and
highlighting, `OPERATOR_NAMES` for completion).

## Acceptance criteria
- [x] The session logic (`ReplSession`: submit rule, let saving and redefinition, span mapping, meta
      commands, did-you-mean) has no terminal dependency and is unit-tested on its own, including:
      an error in the new input points at `<repl:N>` line 1; an error in a saved let points at its
      own snippet; a failed input saves nothing; redefining `x` removes its users and says so; two
      `let x` in one input are still `DS1003`; a file created after a `DS3001` is found on the next
      submission.
- [x] Validator tests for the submit rule: `;` inside a string or comment, unbalanced brackets, a
      blank line, a meta command.
- [x] `assert_cmd` tests in piped mode: lets across inputs, `.schema`, `.tables`, `.format csv`, an
      unknown meta command with did-you-mean, exit codes 0 and 1.
- [x] Ctrl-C cancels a running query without exiting; Ctrl-D exits. (Checked by hand; say so in the
      task summary.) *Checked by driving `ds` through a pseudo-terminal, not by an automated test:
      Ctrl-C during a 14-way self-join prints `query cancelled`, the query's CPU use stops within
      about 2 s, the saved lets still work; Ctrl-C on an empty prompt prints the hint and on
      multi-line input discards it silently; the continuation prompt, blank-line submit, history
      file and Ctrl-D (exit 0) behave as specified.*
- [x] D-048 is recorded and LANGUAGE.md §2 notes the REPL exception to `DS1003` (done in the
      design review). Update both if implementation forces a change.

## Carried over
- [x] (from M1-10) `ds run` prints `DS5001` with `chart_not_drawn` in `crates/datascript-cli/src/main.rs` (once per program run, at the first non-`table` render, with a note counting the others). In the REPL, one submitted input is one program run: call the same helper on each input's results, and add a test that `render table` prints nothing and two charts in one input print one `DS5001`.
- [x] (from M1-07) The session caches failed schema lookups too (D-043). Call `Session::invalidate_schemas` before each submission (or on a `.reload` command) so a file created or edited after an error is read again. *(Settled above: before every submission and `.schema`; no `.reload`.)*

## Follow-ups
- Handed to M1-13: the "… N more rows (show them with --max-rows N)" footer names a startup flag in
  the REPL.
- Informational: implementation choices beyond the plan (reedline 0.49 for the MSRV, the extra
  `nu-ansi-term` dependency, worker-thread checking and running for Ctrl-C, no repeated warnings
  for saved lets, `--table`/`--max-rows` before a command is a usage error) are recorded under
  D-048's implementation notes. Unit tests: `crates/datascript-cli/src/repl/{session,input,editor}.rs`;
  piped end-to-end tests: `crates/datascript-cli/tests/repl.rs`.
