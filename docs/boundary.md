# The boundary: what the testbed promises its callers

Everything a caller may rely on stands here: the binary's verbs and their exit codes, the structure
`--json` answers carry, the environment the verbs read and hand on, the declaration, the events, the
placement endpoint and the crates' public roles. What is not here is internal and may change in any
release. The version rules at the end are over this document.

## The binary

- **One binary, `bmo-testbed`,** whose first word is the verb. `--help` or `-h`, alone or after a
  verb, prints the usage and exits 0.
- **Every verb's answer is English,** thin by default; `--json`, where a verb takes it, gives the
  structure instead, and no verb carries a third width between the two.
- **A verb handed the wrong thing** names that one thing in a line, points at `--help`, and exits 2.
- **A checkout of the testbed runs its own.** Run inside a checkout of this repository, `suite`,
  `check` and `template` build that checkout's own binary through the Cargo verb, in the wide lane,
  and hand it the same words, so the code that checks a change is the code the change carries. A
  checkout whose binary does not build ends with a failed verdict saying nothing was tested.
- **The ceiling is 30,000 characters.** A verb that would answer with more cuts to whole items and says
  in one line how many of the things asked for are in the answer, how many are not, and how to ask for
  the rest. On a `--json` answer that line goes to standard error, so standard output stays valid JSON.
  Where one item alone is over the ceiling, nothing comes back but the line.

## Exit codes

| Code | Meaning |
|---|---|
| 0 | Done, or passed |
| 1 | Ran and failed: a test, a check, a step, a deployment |
| 2 | Refused: words that do not read, a declaration that does not read or names another major, a change that cannot be read, a refusal a verb names |
| 75 | Not run: the turn did not come, a lock did not come, a placed machine dropped the run, or a suite left units unstarted and failed nothing. Run the same command again |
| any other | `run` only: the command's own exit code, passed through |

## The verbs

### `run`

```
bmo-testbed run [--narrow] [--adapter <name>] [--guard <seconds>] [--wait <seconds>] -- <command…>
```

- Runs the command in the wide lane's turn, or the narrow lane's with `--narrow`, read by the language
  that claims it or the one `--adapter` names (`bun`, `cargo`, `node`, `browser`, `plain`). `--guard`
  replaces the thirty-minute run guard, `--wait` the wait for the turn; both take positive whole
  seconds.
- Placed first where the placement endpoint answers so (「Placement」), unless inside a held turn.
- Prints the waits, the one `testbed:` result line, and under a failure at most sixty failure lines.
- Exits 0 where it passed, the command's own code where it failed, 75 where it was not run, 2 where its
  words do not read (no `--`, no command, a flag missing its value, an unknown adapter, which the line
  names with the adapters there are), and 1 on an internal error.

### `status`

```
bmo-testbed status [--json]
```

- Prints each lane, its width, its holder or nobody, and its waiters in order with their age and their
  front, next and kept marks; then the close line of the repository it is run in. Exits 0.
- **`--json`** prints one object:
  - `wide`, `narrow`: each a queue, with `holder` (an entry or null) and `waiting` (a list of entries,
    in the order they will be served);
  - `close_line`: the repository's close line, with `holder` (a close entry or null) and `waiting`, or
    null outside a repository.
- **An entry** carries:

  | Field | Type | Meaning |
  |---|---|---|
  | `place` | string or null | The agent place of the job that started it, from `BMO_AGENT_PLACE` |
  | `checkout` | string | The checkout it was started in |
  | `command` | string | The command, as one line |
  | `asked` | integer | When it asked for the turn, milliseconds since the epoch |
  | `since` | integer, optional | When the turn came; present on a holder |
  | `pid` | integer | Its process |
  | `kept` | boolean, optional | A place kept for a run that ended not run; absent means false |
  | `front` | boolean, optional | Standing at the front; absent means false |
  | `next` | boolean, optional | Standing next after the front; absent means false |

- **A close entry** carries `place`, `checkout`, `asked`, `since` and `pid` as an entry does.

### `classify`

```
bmo-testbed classify [--json] <shell line>
```

- Reads one shell line, given whole as one word, as the shell splits it (`docs/testbed.md`, 「A shell
  hook sends a job's heavy command to it」), and answers whether it holds a heavy command started
  outside the testbed.
- Thin: `light`, or `heavy <language>` followed by the line to run instead.
- **`--json`:** `heavy` (boolean), `language` (string or null), `command` (the heavy command's words, or
  null), `instead` (the line to run instead, or null).
- Exits 0 for light, 1 for heavy, 2 where its words do not read. A line the parser cannot read is light.

### `suite`

```
bmo-testbed suite [--since <commit>] [--again] [--guard <seconds>] [--file-guard <seconds>] [--turn-wait <seconds>] [<word>…]
```

- Runs the repository's units (`docs/suite.md`): all of them, or those the change since `--since`
  reaches. A word naming a path narrows the files; a word that is a unit's name exactly picks that
  unit; every other word goes to each file's runner. `--again` runs what failed or never started once
  more.
- Writes its `suite:` lines and each unit's output to standard output, to be redirected to a file.
- Exits 0 where it passed, 1 where anything failed or was cut, 75 where it was not run or left units
  unstarted and failed nothing, 2 where its words or the declaration do not read.

### `count`

```
bmo-testbed count [--since <commit>]
```

- Counts the tests the range removed and whether each is accounted for (`docs/checks.md`, 「The test
  count」), and prints the tests and Rust tests the tree holds. Exits 1 where a removal is not
  accounted for.

### `log`

```
bmo-testbed log <log file> [--json]
```

- Reads a suite's or a single run's log: totals, verdict, the units that failed and never started, and
  each failing test with the unit it stands in. **`--json`:** `passed`, `failed`, `skipped`,
  `verdict`, `failed_units`, `unstarted_units`, and `failures`, each with `test` and `unit`. Exits 1
  where the log holds a failure.

### `template`

```
bmo-testbed template make|drop|sweep <name>
```

- Makes the named test database template through the declared migrate command, drops it with its pool,
  or sweeps every other template made more than a day ago that no living process holds
  (`docs/suite.md`, 「The test database」). Exits 1 where the server or the migrator fails.

### `check`

```
bmo-testbed check [--since <commit>] [--log <path>] [<file…>]
bmo-testbed check quick
bmo-testbed check wording|layout|dependencies
bmo-testbed check korean [--kinds] [--settle] [<prefix>]
bmo-testbed check dashes <file…> [--write]
```

- With no part named, the change check (`docs/checks.md`, 「The change check」): exits 1 where any step
  failed, 2 where the change could not be read.
- With a part named, that quick check alone, or every enabled one for `quick`. `korean --settle` lowers
  the ledger; `dashes <file…>` rewrites the dashes between words in the markdown files named, only with
  `--write`, and otherwise prints each line it would change. Each exits 1 where it finds anything, and
  2 where the declaration does not enable it or names none of its inputs.

### `imports`

```
bmo-testbed imports [<file…>] [--fix]
```

- Names every relative import a move left broken and where it would go; `--fix` rewrites them all or
  none (`docs/checks.md`). Exits 1 while any is broken or refused, 0 where everything resolves.

### `cargo`

```
bmo-testbed cargo <cargo arguments…>
```

- Runs Cargo through the shared build directory with the checkout's own crates apart in it, in the wide
  lane where the command compiles, run again where the kernel killed it for want of memory, then
  removes what the build superseded. Exits with Cargo's own code, 75 where the turn did not come.

### `prune`

```
bmo-testbed prune [--dry-run] [--older-than <days>] [--incremental-older-than <hours>]
bmo-testbed prune --all
```

- Prunes the shared Cargo build directory (`docs/deploy.md`, 「The Cargo build directory」). Exits 75
  where a lock did not come, 2 for a unit span of a day or less or a disk that keeps no access times.

### `full`

```
bmo-testbed full [--due <HH:MM>]
```

- Starts the daily whole run of the repository in a unit of its own and ends, or with `--due` only
  where none started since that time of day (`docs/suite.md`, 「The daily whole run」). Inside its own
  unit it runs to its end and exits 1 where it failed. Exits 2 where another run holds the lock.

### `deploy`

```
bmo-testbed deploy [--commit <sha>] [--dry-run] [--no-check] [--before-start <command>]
```

- Puts the commit live (`docs/deploy.md`). Outside a deployment's unit it starts one, prints its name
  and the command that reads its journal, and exits 0; inside it, it runs and exits 0 where the commit
  went live, 1 where it failed or was rolled back. Its refusals exit 2 in the caller's own process.
  `--dry-run` says what it would do and changes nothing.

### `backups`

```
bmo-testbed backups
```

- Sends and rotates the backups (`docs/deploy.md`, 「Sending backups away」). Exits 1 where any backup
  failed to send, the rest sent all the same.

## The environment

| Variable | Read by | Meaning |
|---|---|---|
| `BMO_DATA_DIR` | every verb | The testbed's data root: logs under `testbed/`, locks under `locks/`. Default `$HOME/.local/share/bmo/data` |
| `TMPDIR` | the turn | Where the lane locks stand. Default `/tmp` |
| `BMO_TESTBED_LOCK` | the turn | Another wide lane lock, the narrow one beside it |
| `BMO_TESTBED_HELD` | the turn | Set by a holder for its children: its process number. A command finding it holding its lane runs at once |
| `BMO_TESTBED_FRONT` | the turn | `1` stands the caller at the front, unless it is inside a job's call |
| `BMO_TESTBED_NEXT` | the turn | `1` stands the caller next after the front |
| `BMO_TESTBED_TIMES` | the turn | A file each turn taken adds a line to: the command, its lane, its wait and its run |
| `BMO_TEST_PARALLEL` | the turn, the suite | The wide lane's width; set by the turn for a language's runner |
| `CLAUDE_PID` | the turn, `deploy` | The job's agent process: a caller with one is inside a job's call, its holder is watched, and `deploy` refuses it |
| `BMO_AGENT_PLACE` | the turn | The agent place a ticket names |
| `BMO_TESTBED_PLACEMENT` | `run`, `suite` | The placement endpoint, over the declaration's |
| `BMO_TESTBED_EVENT_HOOK` | every verb that sends events | The event hook, over the declaration's, and the hook for a verb run outside a repository |
| `BMO_SUITE_MEMORY_GB` | the suite | The fence's memory ceiling |
| `BMO_TEST_POSTGRES` | the suite, `template`, `scratch-database` | The test database server's address; set by the suite for every process it starts |
| `BMO_TEST_TEMPLATE` | a consumer's code under test | Set by the suite where migrations are declared: the template whose pool a file claims its slot from |
| `BMO_TEST_TEMPLATE_URL` | the declared migrate command | Set by `template make`: the database to migrate |
| `BMO_TEST_HOME` | a consumer's code under test | Set by the test kit's test home: the file's home. A process that carries it is a test |
| `BMO_FULL_CHECK_DIR` | `full` | The daily whole run's tree |
| `BMO_DEPLOY_DIR` | `deploy`, every process of a release | The release a process reads; handed on to what it starts |
| `BMO_DEPLOY_UNIT` | `deploy`, `full` | Set inside a deployment's or a whole run's unit: its name. A run finding it runs where it is |
| `BMO_DEPLOY_PROGRESS` | `deploy` | Another progress record file |
| `BMO_UNIT` | `deploy` | Another `bot` service unit name, for a rehearsal |
| `BMO_BACKUP_DIR` | `deploy`, `backups` | Another backup directory, over the declaration's |
| `BMO_BACKUP_SNAPSHOT_DIR` | the declared snapshot and verify commands | Set by the backup: the directory to write into or read from |
| `BMO_CARGO_TARGET_DIR` | `cargo`, `prune`, the suite | The shared Cargo build directory. Default `$HOME/.local/share/bmo/cargo-target` |
| `XDG_CONFIG_HOME` | `deploy` | Where the user units are installed |

## The declaration

`testbed.toml` at the repository's root, its whole schema in `docs/declaration.md`. In short:

- `testbed`: the major targeted;
- `[test]` with `[test.database]`: the languages, files, guards, fence and setup of the suite;
- `[projects]` and `[tools]`: units found by their own files;
- `[check]` with `[[check.step]]`: the quick checks enabled and their inputs, and the static steps;
- `[build]` with `[[build.step]]`: the release build;
- `[deploy]` with `[[deploy.step]]` and `[deploy.backup]`: the deploy tree, data root, env links, units,
  health, declared steps and backups;
- `[runner]`: the placement endpoint;
- `[events]`: the event hook.

Units are declared by their files: every unit file the `deploy.units` globs find that carries an
`X-BMO-Deploy=` mark (`bot`, `restart`, `binary`, `timer`, `link`, `retire`) is the deployment's to
keep (`docs/deploy.md`, 「The unit links」).

## Events

- **What the testbed tells instead of telling anyone itself.** A fact a person or an agent should hear,
  a deployment failed, a backup did not send, goes to the event hook; what the hook does with it is the
  consumer's.
- **The hook** is the command `BMO_TESTBED_EVENT_HOOK` or the declaration's `[events] hook` names, run
  through `sh -c` with the event's name as its first argument and the payload as one JSON object on
  standard input. With no hook, the event is written on the verb's own output and nothing more.
- **The hook never changes a verb's outcome.** One that fails or overruns its time, thirty seconds
  unless declared, is said on a line and the verb goes on.
- **The payload:**

  | Field | Type | Meaning |
  |---|---|---|
  | `version` | integer | The payload's shape, 1 |
  | `event` | string | The event's name |
  | `at` | string | When, RFC 3339 |
  | `repository` | string or null | The repository's declared name |
  | `root` | string or null | The checkout or deploy tree it happened in |
  | `commit` | string or null | The commit concerned |
  | `summary` | string | One English line saying what happened |
  | `detail` | object | The event's own fields, below |

- **The events and their `detail`:**

  | Event | `detail` |
  |---|---|
  | `deployed` | `commit`, `previous`, `flaky` (tests with `test` and `unit`), `warnings` (lines) |
  | `deploy-failed` | `commit`, `stage`, `reason`, `rolled_back` (boolean), `journal` |
  | `deploy-rolled-back` | `failed_commit`, `back_to`, `reason`, `answers` (boolean), `journal` |
  | `backup-upload-failed` | `backups`, each with `name`, `step` and `said` |
  | `releases-pinned` | `releases`, each with `release` and `processes`, each with `pid`, `unit` and `command` |
  | `cargo-target-over-ceiling` | `bytes`, `ceiling_bytes`, `checkouts` |
  | `full-suite-failed` | `head`, `log`, `step` (the failing setup or static step, or null), `failures`, each with `unit` and `package` |
  | `suite-flaky` | `tests`, each with `test` and `unit`, `units`, `log` |

## Placement

- **Where a heavy run goes is the placement endpoint's answer;** the default is local, so a testbed
  with no endpoint runs everything where it was started.
- **The endpoint** is an HTTP address, `BMO_TESTBED_PLACEMENT` or the declaration's `[runner]
  placement`. Before asking for its turn, a `run` or a `suite` not inside a held turn posts one JSON
  object to it:

  | Field | Type | Meaning |
  |---|---|---|
  | `checkout` | string | The caller's checkout |
  | `remote` | string | The checkout's remote address |
  | `commit` | string | The checkout's `HEAD` |
  | `command` | list of strings | The command's words |
  | `lane` | string | `wide` or `narrow` |
  | `front`, `next` | booleans | Where the caller stands |
  | `wait_seconds`, `guard_seconds` | integers | The caller's wait and guard |

- **The answer** is one JSON object. Its `place` is `local`, and the command runs here; or `remote`,
  with `machine`, the placed machine's label, and `run`, the address of the run there. The caller then reads the run address as an
  event stream: `output` events carry the run's output lines, and one `result` event carries the
  result as the testbed's result line and the exit code.
- **Anything else is local:** no endpoint, one that does not answer within five seconds, an error, or an
  answer that does not read. The caller says so on a line and runs here.
- **A stream that ends with no `result`** ends the run as not run, exit code 75.
- **A run started on a placed machine is never placed again:** the remote side runs it inside a held
  turn of its own.
- **In the crates the placement is a trait** with one method, asked with the run's words and answering
  here or elsewhere with an exit code; the local placement, always here, is the default, and the HTTP
  placement is a member of it.

## The crates

- **Each crate's package is named for its folder** after `bmo-testbed-`: `bmo-testbed-core`,
  `bmo-testbed-scratch-database` and so on.
- **`core`:** the lanes, the turn, the queue, the close line, the run and its result, the languages,
  memory limits, the declaration reader and the policy loader. Its public types are the ones the status
  fields above serialise: the lane, the queue, the entry, the close line and its entry; the turn's ask
  and the turn held; the run's result; the language type every language is a member of.
- **`suite`:** the whole-suite run, change selection, the static and quick checks, the Cargo verb, the
  superseded removal and the prune.
- **`deploy`:** releases, the link flip, unit links, the deploy run, the progress record, rollback
  through the tree's and the services' hands, backups.
- **`scratch-database`:** per-test databases copied from a template the migrator a test hands in
  makes (`docs/suite.md`, 「The test database」): a scratch database dropped with its guard, its
  address and connection, and the test server's address; a consumer's crate may take it as a
  dev-dependency.
- **`testkit`:** the TypeScript package `@bmo-testbed/testkit`, which a consumer may take as a
  dev-dependency (`docs/suite.md`, 「The test home」):
  - `test-stand-ins` answers the colours question on loading, and stands in for a module the consumer
    names, refusing whatever is asked of it; it is preloaded first.
  - `test-home` sets up the file's home from the consumer's ask, the variables to empty and remove, the
    programs to stand in for and the directories to name, and answers the home and its `bin`;
    the consumer's own preload calls it once.
  - `scratch` hands a test a directory taken away as its file ends; `ids` holds the fixed ids.
- **`test-support`:** the fixed ids a test names a user, a room, a message, an account, a worker and a
  unit by, the same values as the test kit's `ids`; a consumer's crate may take it as a
  dev-dependency.

## Versions

- **One version, the workspace's,** in semantic versioning, over everything this document states.
- **A major release** removes or renames a verb, a flag or an environment variable; changes what an
  exit code, a status field, an event, a payload field or a declaration key means; removes a status
  field, an event, a payload field or a declaration key; makes an optional declaration key required;
  or changes the placement endpoint's request or answer incompatibly.
- **A minor release** adds a verb, a flag, an environment variable, an optional declaration key, a
  status field, an event, a payload field, a language or a quick check, or widens what a verb accepts.
- **A patch release** changes nothing this document states.
- **The declaration names its major** in `testbed`; a testbed of another major refuses it with exit
  code 2, naming both, and accepts a declaration of its own major whatever minor wrote it, refusing
  only keys it does not know.
- **The payload's `version`** moves only with a major release.
