# The boundary: what the testbed promises its callers

Everything a caller may rely on stands here: the binary's verbs and their exit codes, the structure
`--json` answers carry, the environment the verbs read and hand on, the declaration, the event hook,
the HTTP service and its routes, the harness plugin and its tools, the placement endpoint and the
crates' public roles. What is not here is internal and may change in any release. The version rules
at the end are over this document.

- **The testbed is an upstream** in the sense of bmo-chat's `docs/conventions.md`: a harness takes it
  as its first entry point through its MCP server, with no other unit running. Its routes, members,
  tools and verbs name no caller; who stands in a lane is the caller's opaque `scope`.
- **The wire format and the API design are the conventions' own,** and are not restated here: keys
  in camelCase, times in epoch milliseconds, the error shape of `code` and `error`, pages of `items`
  and `more`, and one event stream.
- **`docs/openapi.json` is the contract of the routes and the tools,** generated from the client
  crate's types and the plugin's tool table; a test fails where the committed document differs from
  what the code generates.

## The binary

- **One binary, `bmo-testbed`,** whose first word is the verb, or a noun `bmo` dispatches to it
  (「The nouns `bmo` serves」). `--help` or `-h`, alone or after a verb, prints the usage and exits 0.
- **Every deployment links `bmo-testbed` into `.local/bin` under the home directory,** before the flip,
  making the folder where it does not stand and replacing whatever stood under that name. `bmo` finds a
  unit command beside itself there first (bmo-cli's `docs/api.md`, 「The unit command」), so every noun
  below is served wherever `bmo` is. The link names the deploy tree's own link, which stands across
  releases, so it names whichever release is live and never dangles once an older one is pruned.
- **Every verb's answer is English,** thin by default; `--json`, where a verb takes it, gives the
  structure instead, and no verb carries a third width between the two.
- **A verb handed the wrong thing** names that one thing in a line, points at `--help`, and exits 2.
- **A checkout of the testbed runs its own.** Run inside a checkout of this repository, `suite`,
  `check` and `template` build that checkout's own binary through the Cargo verb, in the wide lane,
  and hand it the same words in the checkout's root, the directory they were given in named in
  `BMO_TESTBED_CALLED_FROM`, so the code that checks a change is the code the change carries. A
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
bmo-testbed run [--narrow] [--adapter <name>] [--guard <seconds>] [--wait <seconds>] [--json] -- <command…>
```

- Runs the command in the wide lane's turn, or the narrow lane's with `--narrow`, read by the language
  that claims it or the one `--adapter` names (`bun`, `cargo`, `node`, `browser`, `plain`). `--guard`
  replaces the thirty-minute run guard, `--wait` the wait for the turn; both take positive whole
  seconds.
- Placed first where the placement endpoint answers so (「Placement」), unless inside a held turn.
- Every run that is not placed elsewhere is a test run of kind `command` in the record (「The
  record」), written as its turn comes and ended with its counts.
- Prints the waits, the one `testbed:` result line, and under a failure at most sixty failure lines.
- **`--json`** prints those lines on standard error instead, and on standard output one command run
  (「Members」) once the run has ended; the exit code is the same.
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
  - `queues`: the page of both lanes exactly as `GET /queues` answers it (「Members」);
  - `closeLine`: the repository's close line, with `holder` (a close entry or null) and `waiting` (a
    list of close entries, in the order they will be served), or null outside a repository.
- **A close entry** carries `scope`, `checkout`, `askedAt`, `heldAt` and `pid` as a queue entry does.

### `classify`

```
bmo-testbed classify [--json] [--takes-turn <command>]… <shell line>
```

- Reads one shell line, given whole as one word, as the shell splits it (`docs/testbed.md`, 「A shell
  hook sends a harness's heavy command to it」), and answers whether it holds a heavy command started
  outside the testbed.
- **`--takes-turn`** names a command that takes the turn itself, as its program and the words it opens
  with, one shell line, say `"tool cargo"`; a command opening that way is let through, with
  everything after it in the same command. It may be given again.
- Thin: `light`; or `heavy <language>` and, on the next line, the line to run instead: the running
  binary's `run --` followed by the heavy command's words, quoted as the shell reads them.
- **`--json`:** `heavy` (boolean), `language` (string or null), `command` (the heavy command's words, or
  null), `instead` (the line to run instead, or null).
- Exits 0 for light, 1 for heavy, 2 where its words do not read. A line the parser cannot read is light.

### `suite`

```
bmo-testbed suite [--since <commit>] [--again] [--guard <seconds>] [--file-guard <seconds>] [--turn-wait <seconds>] [--json] [<word>…]
```

- Runs the repository's units (`docs/suite.md`): all of them, or those the change since `--since`
  reaches. A word naming a path narrows the files; a word that is a unit's name exactly picks that
  unit; every other word goes to each file's runner. `--again` runs what failed or never started once
  more.
- Writes its `suite:` lines and each unit's output to standard error, to be redirected to a file, and
  the same to a log of its own under the data root's `testbed/`.
- Every run is a test run of kind `suite` in the record (「The record」), its log the run's own.
- **`--json`** prints one command run (「Members」) on standard output once the run has ended, its log
  the run's own; the exit code is the same.
- **With `--again`,** a first run that failed or left units unstarted says `suite: once more`, naming
  them, runs them once more by name and ends as that second run ends; the tests that passed only the
  second time are written down and sent as a `suite-flaky` event (`docs/suite.md`, 「What failed runs
  once more」). The suite's own flags come before its words.
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
  `verdict`, `failed_units`, `unstarted_units`, and `failures`, each with `test` and `unit`, the unit
  null where the log names none for it. Exits 1 where the log holds a failure.

### `template`

```
bmo-testbed template make|drop|sweep <name>
```

- Makes the named test database template through the declared migrate command, drops it with its pool,
  or sweeps every other template made more than a day ago that no living process holds
  (`docs/suite.md`, 「The test database」). Exits 1 where the server or the migrator fails.

### `check`

```
bmo-testbed check [--since <commit>] [--log <path>] [--json] [<file…>]
bmo-testbed check quick
bmo-testbed check wording|layout|dependencies|cargo-profile
bmo-testbed check language [--kinds] [--settle] [<prefix>]
bmo-testbed check dashes <file…> [--write]
```

- With no part named, the change check (`docs/checks.md`, 「The change check」): exits 1 where any step
  failed, 2 where the change could not be read. **`--json`** prints its lines on standard error
  instead, and on standard output one command run (「Members」) once the check has ended: its log, the
  waits of its steps added up, and its whole time.
- With a part named, that quick check alone, or every enabled one for `quick`. `language --settle`
  lowers the ledger, and `korean` names the same part; `dashes <file…>` rewrites the dashes between
  words in the markdown files named, only with `--write`, and otherwise prints each line it would
  change. Each exits 1 where it finds anything, and 2 where the declaration does not enable it or
  names none of its inputs.

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
bmo-testbed deploy [--commit <sha>] [--message <why>] [--dry-run] [--no-check] [--before-start <command>] [--force-live]
```

- Puts the commit live (`docs/deploy.md`). `--message` is the reason the deploy is made, written into
  the record and the `deployed` event; the commit's subject stands where none is given. Outside a
  deployment's unit it starts one, prints its name
  and the command that reads its journal, and exits 0; inside it, it runs and exits 0 where the commit
  went live, 1 where it failed or was rolled back. Its refusals exit 2 in the caller's own process.
  `--dry-run` says in the caller's own process what it would do, changes nothing and starts no unit.
  `--force-live` goes on past a declared `quiet` command that still names work under way once its wait is
  over (`docs/deploy.md`, 「What the deploy run does, in order」), saying the work it cuts.

### `backups`

```
bmo-testbed backups
```

- Sends and rotates the backups (`docs/deploy.md`, 「Sending backups away」). Exits 1 where any backup
  failed to send, the rest sent all the same.

### `backup`

```
bmo-testbed backup snapshot
bmo-testbed backup media
bmo-testbed backup drill
bmo-testbed backup prune [--dry-run]
```

- Backs up what the settings file under the data root names (`docs/backup.md`): `snapshot` the
  databases and the data folders, sent and rotated; `media` the media sets and the large files; `drill`
  the newest snapshot restored and counted; `prune` the rotation alone, `--dry-run` saying what it would
  remove.
- Thin: a line for each dump, archive, send and removal, with its bytes and seconds, and one closing line.
- Exits 0 where every step passed, 1 where any failed, the rest done all the same, 2 where the settings
  file is missing or does not read, and 75 where another run of the same word holds its lock; `prune`
  shares the snapshot's.

### `serve`

```
bmo-testbed serve [--listen <address>] [--token <token>]
```

- Serves the HTTP service (「The HTTP service」) at the address, the policy's loopback address where
  none is named, behind the bearer token, `BMO_TESTBED_TOKEN` where `--token` names none. It runs
  until it is stopped, and exits 0 on a stop signal.
- Exits 2 where no token is named or the address does not read, and 1 where the address cannot be
  bound or the record cannot be opened.

### `mcp`

```
bmo-testbed mcp [--service <url>] [--token <token>]
```

- Serves the harness plugin's MCP server on standard input and output (「The harness plugin」): the
  reading tools call the HTTP service at the address, `BMO_TESTBED_SERVICE` where `--service` names
  none and the policy's loopback address where neither does, behind the token, `BMO_TESTBED_TOKEN`
  where `--token` names none; the running tools start this same binary's verbs.
- Exits 0 when the harness closes the session, and 1 where it ends in a failure.

### The nouns `bmo` serves

`bmo-testbed nouns` prints the nouns `bmo` dispatches to this binary, one a line, each the words a
caller types after `bmo` (bmo-cli's `docs/api.md`, 「The unit command」). A noun is the platform's
name for a verb: `bmo` hands the noun's words on whole, so the binary takes them where it takes the
verb's own name, and each noun takes what that verb takes.

| Noun | Takes | Verb it runs |
|---|---|---|
| `test suite` | the words `suite` takes | `suite` |
| `test count` | the words `count` takes | `count` |
| `test log` | the words `log` takes | `log` |
| `test template` | the words `template` takes | `template` |
| `test full` | the words `full` takes | `full` |
| `check` | the words `check` takes | `check` |
| `imports` | the words `imports` takes | `imports` |
| `cargo` | the words `cargo` takes | `cargo` |
| `cargo prune` | the words `prune` takes | `prune` |
| `deploy commit` | the words `deploy` takes | `deploy` |
| `deploy backups` | the words `backups` takes | `backups` |
| `backup snapshot` | nothing | `backup snapshot` |
| `backup media` | nothing | `backup media` |
| `backup drill` | nothing | `backup drill` |
| `backup prune` | the words `backup prune` takes | `backup prune` |

## The environment

| Variable | Read by | Meaning |
|---|---|---|
| `BMO_DATA_DIR` | every verb | The testbed's data root: logs and the record under `testbed/`, locks under `locks/`, the data backup's settings under `env/`. Default `$HOME/.local/share/bmo/data` |
| `TMPDIR` | the turn | Where the lane locks stand. Default `/tmp` |
| `BMO_TESTBED_LOCK` | the turn | Another wide lane lock, the narrow one beside it |
| `BMO_TESTBED_HELD` | the turn | Set by a holder for its children: its process number. A command finding it holding its lane runs at once |
| `BMO_TESTBED_FRONT` | the turn | `1` stands the caller at the front, unless it is inside a harness's call |
| `BMO_TESTBED_NEXT` | the turn | `1` stands the caller next after the front |
| `BMO_TESTBED_TIMES` | the turn | A file each turn taken adds a line to: the command, its lane, its wait and its run |
| `BMO_TEST_PARALLEL` | the turn, the suite | The wide lane's width; set by the turn for a language's runner |
| `CLAUDE_PID` | the turn, `deploy` | The process of the harness whose call the caller runs in: a caller with one is inside a harness's call, its holder is watched, and `deploy` refuses it |
| `BMO_TESTBED_SCOPE` | the turn, the record | The caller's opaque scope, which a ticket, a holder and a test run carry and the testbed never reads |
| `BMO_TESTBED_TOKEN` | `serve`, `mcp` | The HTTP service's bearer token |
| `BMO_TESTBED_SERVICE` | `mcp` | The HTTP service's address the reading tools call |
| `BMO_TESTBED_PLACEMENT` | `run`, `suite` | The placement endpoint, over the declaration's |
| `BMO_TESTBED_EVENT_HOOK` | every verb that sends events | The event hook, over the declaration's, and the hook for a verb run outside a repository |
| `BMO_SUITE_MEMORY_GB` | the suite | The fence's memory ceiling |
| `BMO_TEST_POSTGRES` | the suite, `template`, `scratch-database` | The test database server's address; set by the suite for every process it starts |
| `BMO_TEST_TEMPLATE` | a consumer's code under test | Set by the suite where migrations are declared: the template whose pool a file claims its slot from |
| `BMO_TEST_TEMPLATE_URL` | the declared migrate command | Set by `template make`: the database to migrate |
| `BMO_TEST_HOME` | a consumer's code under test | Set by the test kit's test home: the file's home. A process that carries it is a test |
| `BMO_FULL_CHECK_DIR` | `full` | The daily whole run's tree |
| `BMO_DEPLOY_DIR` | every process of a release | The release a process reads; handed on to what it starts. `deploy` never takes its deploy tree from it |
| `BMO_TESTBED_CALLED_FROM` | every verb a checkout runs its own of | Set by the hand-over to a checkout's own binary: the directory the words were given in, which a path among them is read from |
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

- **What the testbed tells instead of telling anyone itself.** A fact the caller should hear,
  a deployment failed, a backup did not send, goes to the event hook; what the hook does with it is the
  consumer's.
- **The hook** is the command `BMO_TESTBED_EVENT_HOOK` or the declaration's `[events] hook` names, run
  through `sh -c` with the event's name as its first argument and the payload as one JSON object on
  standard input. With no hook, the event is written on the verb's standard error as one line, its
  name and its payload, and nothing more.
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
  | `deployed` | `commit`, `message` (the reason, the commit's subject where none was given), `previous`, `flaky` (tests with `test` and `unit`), `warnings` (lines) |
  | `deploy-failed` | `commit`, `stage`, `reason`, `rolled_back` (boolean), `journal` |
  | `deploy-rolled-back` | `failed_commit`, `back_to`, `reason`, `answers` (boolean), `journal` |
  | `backup-upload-failed` | `backups`, each with `name`, `step` and `said` |
| `data-backup-failed` | `verb` (`snapshot`, `media` or `prune`), `failures`, each with `step`, `target` and `said` |
| `restore-drill-failed` | `verb` (`drill`), `failures`, each with `step`, `target` and `said`, a count mismatch naming its database, table and both counts |
  | `releases-pinned` | `releases`, each with `release` and `processes`, each with `pid`, `unit` and `command` |
  | `cargo-target-over-ceiling` | `bytes`, `ceiling_bytes`, `checkouts` |
  | `full-suite-failed` | `head`, `log`, `step` (the failing setup or static step, or null), `failures`, each with `unit` and `package` |
  | `suite-flaky` | `tests`, each with `test` and `unit`, `units`, `log` |

## The record

- **What the verbs did is written into one record** on the machine, `record/record.sqlite` under the
  data root's `testbed/`, which every verb writes and the HTTP service reads: the test runs, the
  deployments with their steps and log lines, the restarts, and the events of each.
- **A test run** is written by every `run` that is not placed elsewhere, kind `command`, and by every
  `suite`, kind `suite`, a deployment's gate and the daily whole run among them. It is written once
  its turn has come and ended with its counts; a run ending not run from there on, a suite leaving
  units unstarted and failing nothing, ends `not-run`. A run placed elsewhere is written by the
  machine it runs on.
- **A deployment** is written as the deploy run begins its progress record and changed with it
  (`docs/deploy.md`, 「What a deployment says of itself as it runs」), and every line the deploy run
  says is one of its log lines.
- **A restart** is written by a deployment for every unit it stops or restarts, before it does: the
  unit, the deployment, and one English sentence of why, so the unit reads why it was stopped as it
  starts again.
- **Writing the record never changes a verb's outcome.** A record that cannot be opened or written is
  said on one line of standard error and the verb goes on.
- **What it keeps:** test runs, deployments, their log lines and restarts for the policy's days, and
  events for the policy's window; older ones go as the next one is written.

## The HTTP service

- **It only reads.** Running anything is the verbs'; the routes serve what the record holds and the
  lanes as they stand. `serve` starts it (「`serve`」).
- **Every route takes the bearer token,** `GET /health` included, and answers 401 `unauthorized`
  without it.

### Routes

| Method and path | Answers | Codes beside `unauthorized` |
|---|---|---|
| `GET /health` | the health | |
| `GET /events` | the event stream, or a page of events to `Accept: application/json` | `bad-request`, `gone` |
| `GET /queues` | a page of the two lanes, the wide one first | `bad-request` |
| `GET /test-runs` | a page of test runs, newest first, narrowed by `checkout` | `bad-request` |
| `GET /test-runs/{testRunId}` | one test run | `not-found` |
| `GET /deployments` | a page of deployments, newest first | `bad-request` |
| `GET /deployments/{deploymentId}` | one deployment | `not-found` |
| `GET /deployments/{deploymentId}/log-lines` | a page of the deployment's log lines, oldest first | `bad-request`, `not-found` |
| `GET /restarts` | a page of restarts, newest first, narrowed by `unitName` | `bad-request` |

- **A list pages by `after` and `limit`.** `after` is the id of the last item of the page before, so
  a list newest first goes on to older ids and one oldest first to newer ones; `limit` is the policy's
  default where none is named and never more than its ceiling. The lanes are always two, and answer
  whole up to the limit.
- **A path that names no route** answers 404 `no-route`; a service failure 500 `failed`.

### Members

Every member has one shape wherever it is answered: alone, in a page, inside an event.

- **Health:** `ready` (boolean), `reason` (string or null, null when ready), `version` (the
  workspace's). It is not ready while the record cannot be opened.
- **Queue:** `name` (`wide` or `narrow`), `width` (integer), `holder` (a queue entry or null),
  `waiting` (a list of queue entries, in the order they will be served).
- **Queue entry:** `scope` (string or null, the caller's opaque scope from `BMO_TESTBED_SCOPE`),
  `checkout`, `command` (one line), `pid`, `askedAt`, `heldAt` (null while waiting), and the booleans
  `kept`, `front` and `next`.
- **Test run:** `id`, `kind` (`suite` or `command`), `checkout`, `repositoryName` (string or null),
  `commit` (string or null), `command` (the words run, as one line), `scope` (string or null),
  `unitName` (the unit it runs in, from `BMO_DEPLOY_UNIT`, or null), `pid`, `state`, `passedCount`,
  `failedCount`, `skippedCount`, `logPath`, `startedAt`, `endedAt` (null while it runs).
  - Its `state` is `running`, then one of `passed`, `failed` or `not-run`; a run whose process went
    without ending it is answered `gone`.
- **Deployment:** `id`, `repositoryName` (string or null), `unitName` (the deployment's unit, or
  null), `pid`, `commit` (string or null), `state`, `plan` (the stages it means to pass), `stages`
  (each with `stage` and `startedAt`), `steps`, `rolledBack` (boolean), `stoppedFor` (string or null),
  `reason` (string or null), `startedAt`, `endedAt` (null while it runs).
  - Its `state` is `running`, then one of `live`, `rolled-back`, `failed` or `stopped`; a run whose
    process went without ending it is answered `gone`.
  - `stoppedFor` is the code of why it did not go live: the stage it stopped in (`check`, `backup`,
    `tree`, `build`, `before`, `restart`, `stay-up`), or `refused` for a refusal; null while it runs
    and once it went live. `reason` is the sentence beside its outcome.
- **Deployment step:** `name` (its declared English id), `stage` (string or null), `part` (`snapshot`,
  `pack`, `check` or null), `state` (`ahead`, `running` or `ended`), `exitCode`, `waitedMs`, `spentMs`
  (each null until it ended), `startedAt` (null while ahead), `endedAt` (null until it ended).
- **Log line:** `id`, `at`, `text`.
- **Restart:** `id`, `unitName`, `deploymentId`, `reason`, `at`.
- **Command run,** what `run`, `check` and `suite` print with `--json` and the running tools answer:
  `exitCode`, `logPath` (empty for a run placed elsewhere, whose log stands on that machine),
  `waitedMs`, `spentMs`, and `testRunId` (the test run it wrote, or null for a check or a run placed
  elsewhere).

### The event stream

- **`GET /events` is the service's one stream,** of these kinds, each event's data the member as a
  `GET` answers it at the time:

  | Kind | Data |
  |---|---|
  | `queue.changed` | the queue whose holder or line changed |
  | `test-run.created` | the test run as it began |
  | `test-run.ended` | the test run as it ended |
  | `deployment.created` | the deployment as it began |
  | `deployment.changed` | the deployment after each change of its record |
  | `restart.created` | the restart |

- **A lane's change is seen by the service itself,** which reads the lanes every second of the
  policy's and writes a `queue.changed` event where a holder or a line differs from the last reading.
- **A reader resumes** with `Last-Event-ID` or `after`, and narrows with `kinds`, a comma-separated
  list. The window is the policy's; a resume from before it answers 410 `gone`.
- **To `Accept: application/json`** the same events are a page by `after` and `limit`.
- **A comment line every fifteen seconds** keeps the stream open.

## The harness plugin

- **`bmo-testbed mcp` is the MCP server and `bmo-testbed` its skill,** under `plugin/skills/`, telling
  a harness when and how to use it. A harness's first use is to run its heavy test or build command
  through `testbed_run`, so it waits in the machine's one queue, and to read the run's counts and log
  with `testbed_test_run`.
- **Its tools are exactly these,** each a thin call through the client or of one verb, their inputs and
  outputs stated under `x-tools` in `docs/openapi.json`:

  | Tool | Calls | Answers |
  |---|---|---|
  | `testbed_queues` | `GET /queues` | a page of queues |
  | `testbed_test_runs` | `GET /test-runs` | a page of test runs |
  | `testbed_test_run` | `GET /test-runs/{testRunId}` | a test run |
  | `testbed_deployments` | `GET /deployments` | a page of deployments |
  | `testbed_deployment` | `GET /deployments/{deploymentId}` | a deployment |
  | `testbed_run` | `bmo-testbed run --json --`, in the checkout named, in the narrow lane where `queue` names it | a command run |
  | `testbed_check` | `bmo-testbed check --json`, in the checkout named, from `since` where named | a command run |
  | `testbed_suite` | `bmo-testbed suite --json`, in the checkout named | a command run |

- **A tool that fails** answers a tool error whose text is JSON: `status` where a route answered,
  `code` and `error`. Arguments that are not the tool's shape are `bad-request`; a service nothing
  answers at is an error with no `code`.
- **It sends no notifications,** since the service keeps no subscriptions.

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
  with `machine`, the placed machine's label, `bench`, where the caller's copy of its checkout stands
  there, and `run`, the address of the run there:

  | Field of `bench` | Type | Meaning |
  |---|---|---|
  | `destination` | string | The machine as the caller's `ssh` reaches it |
  | `path` | string | The bench directory on that machine |
  | `ssh` | list of strings, optional | The `ssh` program and its options; `["ssh"]` where absent |

- **The caller makes the bench ready** before it reads the run: where `path` holds no repository it
  clones the checkout's remote there, or makes an empty repository where the checkout has none; it
  pushes its `HEAD` there as the branch `bmo-bench` and checks it out detached; and it copies its open
  files over with `rsync`, the files git ignores and the `.git` directory left out. A step that fails
  ends the run as not run, exit code 75, with a line naming the step and what it said.
- **The run address is an event stream,** asked once the bench is ready, which runs the bench's
  declared setup and the command in that machine's own turn. Each `output` event's data is one line of
  the run's output, which the caller writes to its standard error; one `result` event's data is one
  JSON object with `line`, the testbed's result line, which the caller writes to its standard output,
  and `code`, the exit code it ends with.
- **Anything else is local:** no endpoint, one that does not answer within five seconds, an error, or an
  answer that does not read. The caller says so on a line and runs here.
- **A stream that ends with no `result`** ends the run as not run, exit code 75.
- **A run started on a placed machine is never placed again:** the remote side runs it inside a held
  turn of its own.
- **A run inside a held turn, or inside a deployment's or a whole run's unit, is never placed.**
- **In the crates the placement is a trait** in `core` with one method, asked with the run's words and
  answering here or elsewhere with an exit code; the local placement, always here, is the default, and
  the HTTP placement in `cli` is a member of it.

## The crates

- **Each crate's package is named for its folder** after `bmo-testbed-`: `bmo-testbed-core`,
  `bmo-testbed-scratch-database` and so on.
- **`core`:** the lanes, the turn, the queue, the close line, the run and its result, the languages,
  memory limits, the placement, the declaration reader and the policy loader. Its public types are the
  ones the status fields above serialise: the lane, the queue, the entry, the close line and its entry;
  the turn's ask and the turn held; the run's result; the language type every language is a member of.
  The close line of a repository stands where `core` names it from a data root and any directory of a
  checkout, so a consumer holding the line and `status` reading it meet in one file.
- **`cli`:** the `bmo-testbed` binary: each verb's words read and handed to the crates, the
  classification of a shell line, and the HTTP placement.
- **`client`:** `bmo-testbed-client`, the published client: every member, page, query and error code
  of the HTTP service as its types, and the HTTP client calling each route. Its HTTP part is the
  default `http` feature, so a crate taking the types alone builds without it.
- **`record`:** the record (「The record」): opened from a data root, written by the verbs, read by
  the service, each row read back as the client's member.
- **`service`:** the HTTP service over the record and the lanes, which `serve` starts.
- **`plugin`:** the harness plugin: the tool table, the MCP server `mcp` starts, the OpenAPI document
  generated from the client's types and the tool table, and the `bmo-testbed` skill.
- **`suite`:** the whole-suite run, change selection, the static and quick checks, the Cargo verb, the
  superseded removal and the prune.
- **`deploy`:** releases, the link flip, unit links, the deploy run, the progress record, rollback
  through the tree's and the services' hands, backups. The run asks its gate through a trait of its
  own, which the binary answers with the suite crate's static check and suite, so `deploy` depends on
  no other crate of the testbed but `core` and `record`.
- **`backup`:** the data backup (`docs/backup.md`): the settings read into a plan, the stores a
  destination is reached through, the dumps and their counts, the archives, the snapshot, the media,
  the rotation and the drill.
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

- **One version, the workspace's,** over everything this document states: the crates, the binary, the
  health's `version`, the OpenAPI document and the MCP server all answer it.
- **Before its first release the testbed is `0.1.0` and counts revisions,** by the conventions'
  rule: `0.1.0` is the base and no revision, and every later version is one revision more, written
  `0.1.0-rev.<n>`, whatever the change, a breaking one included. Major, minor and patch never move
  before the first release.
- **From the first release on, the version moves by semver.** A major release removes or renames a
  verb, a flag, an environment variable, a route, a member's field or a tool; changes what an exit
  code, a field, an event, a payload field or a declaration key means; removes an event, a payload
  field or a declaration key; makes an optional declaration key or a field required; or changes the
  placement endpoint's request or answer incompatibly. A minor release adds a verb, a flag, an
  environment variable, an optional declaration key, a route, a field, an event, a tool, a payload
  field, a language or a quick check, or widens what a verb accepts. A patch release changes nothing
  this document states.
- **The declaration names its major** in `testbed`; a testbed of another major refuses it with exit
  code 2, naming both, and accepts a declaration of its own major whatever revision or minor wrote it,
  refusing only keys it does not know.
- **The payload's `version`** moves only with a major release.
