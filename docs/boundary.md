# The boundary: what the testbed promises its callers

Everything a caller may rely on stands here: the binary's verbs and their exit codes, the structure
`--json` answers carry, the environment the verbs read and hand on, the declaration, the event hook,
the HTTP service and its routes, the harness plugin and its tools, the hosted deploy tool, the
placement endpoint and the crates' public roles. What is not here is internal and may change in any
release. The version rules at the end are over this document.

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
| 3 | `status --free` only: the machine is loaded past its share, so it takes no run (「`status`」) |
| 75 | Not run: the harness it waited for went away before the turn came, or a placed machine dropped the run. Run the same command again |
| 124 | Ended as hung: the command holding a lane's turn neither wrote nor worked for the silence limit (`docs/testbed.md`, 「A holder whose command falls silent lets go」) |
| any other | `run` only: the command's own exit code, passed through |

## The verbs

### `run`

```
bmo-testbed run [--narrow] [--adapter <name>] [--json] -- <command…>
```

- Runs the command in the wide lane's turn, or the narrow lane's with `--narrow`, read by the language
  that claims it or the one `--adapter` names (`bun`, `cargo`, `node`, `browser`, `plain`), waiting for
  the turn however long the line takes and running the command until it ends, or until it neither writes
  to its log nor works for the silence limit. `--guard <seconds>` and `--wait <seconds>` are read and given no
  effect.
- Placed first where the machine's placement sends it elsewhere (「Placement」), unless inside a held turn.
- Every run that is not placed elsewhere is a test run of kind `command` in the record (「The
  record」), written as its turn comes and ended with its counts.
- Prints the waits, the one `testbed:` result line, and under a failure at most sixty failure lines.
- **`--json`** prints those lines on standard error instead, and on standard output one command run
  (「Members」) once the run has ended; the exit code is the same.
- Exits 0 where it passed, the command's own code where it failed, 75 where it was not run, 124 where it
  was ended as hung, 2 where its words do not read (no `--`, no command, a flag missing its value, an unknown adapter, which the line
  names with the adapters there are), and 1 on an internal error.

### `placed`

```
bmo-testbed placed [--setup] [--checkout <path>…] [--dir <path>] -- <command…>
```

- The far side of a placed run (「Placement」): run in the bench by the placing machine over `ssh`.
  With `--setup` it first runs the declared `[test].setup` steps in the bench, in order, so a fresh
  bench holds what its check and its run read; a step that fails exits 75 with a line naming it. It then
  runs the words as a command of its own, with `BMO_TESTBED_PLACED` set so the run is never placed
  again, and ends with that command's exit code; the command's output is this verb's.
- **`--dir` names the folder of the bench the command runs in,** from the bench's top with `/` between its
  parts: the folder the caller was started in, from its checkout's top. The setup steps still run at the
  bench's top. Where the bench holds no such folder, nothing runs and the verb exits 2 on a line naming
  it.
- **It watches what it runs for silence** (`docs/testbed.md`, 「A holder whose command falls silent lets
  go」): the setup steps and the command pass their output on through pipes it counts, and are handed
  `BMO_TESTBED_HEARD`. A command that neither writes nor works for the bench's silence limit is ended
  whole and the verb exits 124 with the hung line; a setup step ended so fails as any setup step does,
  exit 75. While the command works or touches that file with none of its own output passing for a
  minute, the verb writes one line on standard error saying it still works, so the placing machine hears
  it over the connection.
- **The checkouts the run knows are the ones `--checkout` names:** the bench and the trees copied beside
  it, as 「The placed machine」 names them. They stand in a registry written for this run alone under the
  data root's `testbed/`, handed to the setup and the command as `BMO_TESTBED_CHECKOUTS`, and taken away
  as the run ends; with none named, the run reads this machine's own registry. A registry that cannot be
  written exits 75.
- The words are a whole command line: `bmo-testbed suite --again`, `bmo-testbed cargo build …`, or
  any program. A testbed verb among them takes its own turn here.
- Exits 2 where its words do not read (no `--`, no command) or the bench holds no folder `--dir` names.

### `crate-tests`

```
bmo-testbed crate-tests record <record> <executable> [<argument>…]
bmo-testbed crate-tests run <record> <package>
```

- The suite's own two halves of a crate's tests (`docs/suite.md`, 「Heavy units run in a lane of one」).
- **`record`** is Cargo's runner while the suite builds the crates' tests: it runs nothing, adds one line
  of JSON to the record naming the executable, its arguments, the folder it was started in and the
  variables Cargo set for it, `CARGO`, `CARGO_MANIFEST_DIR`, `CARGO_MANIFEST_PATH`, every `CARGO_PKG_`
  value and `LD_LIBRARY_PATH`, and exits 0; 1 where the record cannot be written.
- **`run`** runs every executable the record names for the package, by its `CARGO_PKG_NAME`, in the
  order they were written, each to its end, from its folder and with its variables laid over the verb's
  own environment, its output passed through with a line naming it ahead on standard output, where the
  harness writes its results. Exits 0 where every one
  passed or there is none, saying so where there is none; the first failing one's code otherwise, or 1
  where one could not be started; and 1 where the record does not read.
- Exits 2 where its words do not read.

### `status`

```
bmo-testbed status [--json | --free [--narrow]]
```

- Prints each lane, its width, a `holding it:` line for each seat held, naming its holder, or one
  saying nobody, its waiters in order with their age and their front and next marks, and a `placed on`
  line for each run this machine placed elsewhere in that lane (「Placement」, 「A run placed elsewhere
  stands here too」), naming the machine, whether it waits or runs there and since when; then the close
  line of the repository it is run in; then the deployment line: a `holding it:` line naming the
  deployment holding it, or one saying nobody, and its waiting deployments in the order the line will
  serve them, those waiting for their deploy root's turn included where they will reach it
  (`docs/deploy.md`, 「The deployment line」), each naming its repository, commit and unit, how long ago it
  asked, `first, by <who>: <why>` where it is marked first, and `for its repository's turn` where it waits
  for its deploy root's turn; then a line for the placed machine the settings name, where they
  name one: whether it answers, with when its outage began where one stands, and where it does, the
  commit its testbed is built from and whether that is this machine's live testbed release, `current` or
  `behind` naming the live one, or a line saying placement is paused where the settings name no
  `destination`, or that they do not read, naming the file (「The placed machine」). Exits 0.
- **`--json`** prints one object:
  - `queues`: the page of both lanes exactly as `GET /queues` answers it (「Members」);
  - `machines`: one object for the placed machine the settings name, none where they name none, with
    `machine` (its label), `reached` (a boolean), `said` (what `ssh` said where it did not answer, else
    null), `outageSince` (when the outage the runs remember began, null where none stands), `release` (the commit its testbed names, null where it was not reached or names none), `live`
    (this machine's live testbed release's commit, null where this testbed runs from no release) and
    `current` (whether `release` is `live`);
  - `placement`: how the settings stand: `placed` where they name a placed machine, `paused` where they name
    no `destination`, `none` where no settings file stands, and `unread` where it does not read;
  - `closeLine`: the repository's close line, with `holder` (a close entry or null) and `waiting` (a
    list of close entries, in the order they will be served), or null outside a repository.
  - `deployLine`: the deployment line, with `holder` (a deployment ticket or null) and `waiting` (a
    list of deployment tickets, in the order they will be served, those waiting for their deploy root's
    turn included), each waiting ticket with `waitsFor` beside its fields: `line` where it stands in the
    line, `root` where it waits for its deploy root's turn.
- **A deployment ticket** carries `words` (what the line names it by, its repository, short commit and
  unit), `repository`, `commit` (the whole commit; the short one where a testbed of an earlier revision
  stood the ticket), `root` (the deploy root, null where an earlier revision stood the ticket), `unit` (null
  where the deployment runs in none),
  `askedAt` (epoch milliseconds), `pid`, `beforeStart` (whether it was asked with a command to run before its
  start), and `first`: null, or `by`, `why` and `at` (epoch milliseconds)
  of the mark that put it first. A ticket a testbed of an earlier revision stood carries its `words` and
  `pid` alone, every other field null.
- **A close entry** carries `scope`, `checkout`, `askedAt`, `heldAt` and `pid` as a queue entry does.
- **`--free`** prints nothing and exits 0 where this machine takes a run of its wide lane now: a seat free,
  nobody waiting for one, and the machine not loaded (`docs/testbed.md`, 「A run on another machine」). Where it
  takes none it prints one line saying why and ends on the code of that reason: 1 where every seat is held or
  waited for, 3 where the machine is loaded past its share. With `--narrow`, the same of the narrow lane. Every
  machine a placement order names is asked this, of the run's lane, before the run goes (「Placement」).

### `close-line`

```
bmo-testbed close-line --checkout <path>…
```

- **Holds the close line of each named checkout's repository** (`docs/testbed.md`, 「The close line」) for a
  close run outside the testbed, from the moment every line is its own until its standard input ends. A path is
  read from the directory the verb runs in, given once for each checkout; checkouts of one repository, clones and
  worktrees alike, stand in its one line once.
- **The lines are taken one after another in the order of their lock files' names,** whatever order the
  checkouts are named in, each waited for however long the line takes, the waiting said every thirty seconds
  on standard error as a close waiting for its line says it.
- **Once every line is held** it prints one line on standard output naming how many it holds and their
  checkouts, and holds them until its standard input is closed or ends; it then lets every line go and exits 0.
- **The lines are held by its process alone,** so they go the moment it ends, however it ends. Its harness's
  process, named in `CLAUDE_PID`, is watched as a close's is: once it is gone the verb is ended and its lines
  go with it.
- Exits 0 once it has let its lines go, 75 where its harness went away before every line came, the lines it
  already held let go and nothing printed on standard output, and 2 where no checkout is given or a path names
  no checkout of a git repository, the line naming the path.

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
bmo-testbed suite [--since <commit>] [--again] [--json] [<word>…]
```

- Runs the repository's units (`docs/suite.md`): all of them, or those the change since `--since`
  reaches. A word naming a path narrows the files; a word that is a unit's name exactly picks that
  unit; every other word goes to each file's runner. `--again` runs what failed once more. Every unit
  runs until it ends, or fails with exit code 124 once it neither writes nor works for the silence limit;
  `--guard`, `--file-guard` and `--turn-wait`, each with seconds, are read and given no effect.
- Writes its `suite:` lines and each unit's output to standard error, to be redirected to a file, and
  the same to a log of its own under the data root's `testbed/`.
- Every run is a test run of kind `suite` in the record (「The record」), its log the run's own.
- **`--json`** prints one command run (「Members」) on standard output once the run has ended, its log
  the run's own; the exit code is the same.
- **With `--again`,** a unit that ran and failed runs once more as soon as it ends, narrowed to what
  failed in it: its first account stands under the `redo` mark, followed by a line opening with
  `suite: once more` naming the unit and what runs, and the unit stands on that second run. The tests
  that passed only the second time are written down and sent as a `suite-flaky` event (`docs/suite.md`,
  「What failed runs once more」). The suite's own flags come before its words.
- Exits 0 where it passed, 1 where anything failed, 75 where it was not run, 2 where its words or the
  declaration do not read.

### `passed`

```
bmo-testbed passed [--commit <sha>] [--json]
```

- Answers from this machine's record (「The record」) the run of the repository's whole suite a
  deployment's gate takes in place of its own (`docs/deploy.md`, 「What the deploy run does, in
  order」, step 3): the newest test run of kind `suite` of the repository it is run in that is `whole`,
  begun on the commit, the checkout's `HEAD` by default, with `clean` true, within the policy's hours,
  where that run passed. It runs nothing and takes no turn.
- Prints one line naming the run: its id, the commit, when it began, its checkout and its log.
  **`--json`** prints the test run (「Members」) instead.
- Exits 0 where such a run stands, 1 where none does, the line saying so, and 2 where its words or the
  declaration do not read or the commit names none.

### `toolchain`

```
bmo-testbed toolchain [--json]
```

- Answers the toolchains the checkout it runs in is built and tested with, each language's own answer
  (`docs/testbed.md`, 「Languages」): for every language that names a command saying its toolchain, that
  command run in the checkout as the testbed runs the language's commands there, a toolchain the Cargo
  home holds found there first, so a toolchain file the checkout carries is read. It runs nothing else
  and takes no turn. A deployment's gate reads it where the gate runs to know the ground it passed on
  (`docs/deploy.md`, 「What the deploy run does, in order」, step 3).
- Prints, for each such language in the order of their names, a line with its name and a colon, then
  what the command printed, or `none` where it did not start or did not end 0. **`--json`** prints one
  object instead, each language's name to what its command printed, or null.
- Exits 0, and 2 where its words do not read.

### `dependents`

```
bmo-testbed dependents [--run]
```

- Lists the checkouts in the machine's registry whose declarations name this repository in `upstreams`
  (`docs/suite.md`, 「The dependents of a major change」), each by its declared name and root.
- **`--run`** runs each consumer's suite in turn through the suite runner, one repository at a time in
  the wide lane, with that consumer's own declaration, log and test run.
- Exits 0 where every consumer passed or there is none, 1 where any consumer's suite failed,
  75 where none failed but a consumer's suite did not run, and 2 where the registry or a consumer's
  declaration does not read.

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
bmo-testbed check --checkout <path>[@<rev>]… [--log <path>] [--json]
bmo-testbed check quick
bmo-testbed check static [--since <commit>]
bmo-testbed check wording|layout|dependencies|cargo-profile|personal-data|shared-versions
bmo-testbed check shared-versions --report
bmo-testbed check language [--kinds] [--settle] [<prefix>]
bmo-testbed check dashes <file…> [--write]
```

- With no part named, the change check (`docs/checks.md`, 「The change check」): exits 1 where any step
  failed, 2 where the change could not be read. **`--json`** prints its lines on standard error
  instead, and on standard output one command run (「Members」) once the check has ended: its log, the
  waits of its steps added up, and its whole time. While a step runs, a line naming it and how long it has
  run is printed on standard error once a minute, with or without `--json`. Started outside a held turn, a deployment's or a whole
  run's unit and a placed machine, it is placed first where the machine's placement sends it elsewhere
  (「Placement」) and runs there as that testbed's change check with the same `--since`, `--json` and
  files; `--log` then names nothing here, and the log is that machine's. Under a failing step it prints
  what failed in that step's output, and a failing check's log is kept apart beside the log, its path on a
  line opening `failing log kept:` before the last line, `log:` (`docs/checks.md`, 「The change check」).
- **`--checkout`**, given once or more, is the change check over several checkouts (`docs/checks.md`,
  「The change check over several checkouts」): each checkout read from the directory the verb runs in, its
  since after an `@` where given, each planned by its own declaration and the set run in one wide seat, every
  line of a checkout's check, its minute line among them, opening with its repository's name and the last
  lines naming each failed repository and its failed steps; a since that is not an ancestor of its checkout's
  `HEAD` is refused, exit 2. It is never given with `--since` or files. It exits 1 where any step failed or the placement
  refuses the set, 2 where a checkout does not read, and 75 where the set was not run. Started outside a
  held turn, a deployment's or a whole run's unit and a placed machine, it goes where the placed machine's
  `combined_order` sends it (「Placement」) and runs there as that testbed's change check over the same
  checkouts, each named by its place from the first checkout's bench and its since by the commit itself;
  `--json` then prints that machine's command run, and the log is that machine's. A `--json` command run of
  the set names its log and carries no test run.
- **`static`** is the static check a deployment's gate passes, alone (`docs/checks.md`, 「The static check」):
  the quick checks, the declared steps the change since the commit reaches, then the workspace's formatting
  and lints, and no test; a declared step that only repeats those formatting and lints is left out, said in
  one line and counted as passed. It exits 1 where a step failed. A deployment whose gate stands on a placed
  machine runs it there before the suite (「The placed machine」).
- With a part named, that quick check alone, or every enabled one for `quick`. `language --settle`
  lowers the ledger, and `korean` names the same part; `shared-versions` holds the Cargo manifests, the
  lock and the packages the package manifests vendor, both against the shared versions as the main branch
  of the repository holding them has them, whatever branch its checkout stands on;
  `shared-versions --report` holds every checkout of the machine's registry to the shared versions, naming
  beside each pinned package what the checkout writes for it, and changes nothing; `dashes <file…>` rewrites the dashes
  between words in the markdown files named, only with `--write`, and otherwise prints each line it would
  change. Each exits 1 where it finds anything, and 2 where the declaration does not enable it or names
  none of its inputs.

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

- Runs Cargo in the checkout's own directory under the build root, in the wide
  lane where the command compiles, run again where the kernel killed it for want of memory, then
  removes what the build superseded. Exits with Cargo's own code, 75 where its harness went away
  before the turn came, and 124 where, holding the turn with its standard output and error both files,
  it neither wrote to them nor worked for the silence limit.
- Given `--locked` or `--frozen`, runs Cargo on a lock whose path packages are brought to the versions
  their manifests name, offline and fetching no package's source, only where no registry or git package
  moved, says on its standard error on one line which path packages moved, or why it left them as the lock
  holds them, and puts the lock back as it found it once Cargo ends (`docs/deploy.md`, 「The Cargo build
  directory」).
- Placed first where the machine's placement sends it elsewhere (「Placement」) when its subcommand is
  `test`, `check` or `clippy`, it holds no `--no-run` and no word names an absolute path, and it was
  started outside a held turn, a deployment's or a whole run's unit and a placed machine: it then runs as
  the `cargo` verb of the testbed there, and its output and exit code are that run's.
- Given a toolchain with a leading `+` and a subcommand whose program that toolchain may be installed
  without, `fmt`'s rustfmt, first gives the toolchain that component where the machine's rustup lacks it
  (`docs/testbed.md`, 「Languages」), saying so on its standard error.

### `prune`

```
bmo-testbed prune [--dry-run] [--older-than <days>] [--incremental-older-than <hours>]
bmo-testbed prune --all
```

- Prunes the shared Cargo build directory (`docs/deploy.md`, 「The Cargo build directory」), waiting for
  each of Cargo's build locks until the build holding it lets go, and removes the target directory of
  every deployment's build tree whose last deployment is three days old (`docs/deploy.md`, 「The
  releases」). Exits 2 for a unit span of a day or less or a disk that keeps no access times.
- Then, while the directory holds more than its ceiling, `BMO_CARGO_TARGET_CEILING_GB` held to what its
  filesystem can hold, removes the rebuildable output its rules name by least recent read until it holds no
  more, and sends `cargo-target-over-ceiling` where it stays over (`docs/deploy.md`, 「The Cargo build
  directory」); a dry run says what it would remove and how far over the directory stands, and `--all`
  enforces nothing.
- Then trims the filesystem holding the build directory with the prune policy's trim command, so the
  blocks it freed go back to the drive a virtual disk stands on, and sends `disk-trim-failed` where the
  trim does not run (`docs/deploy.md`, 「The Cargo build directory」); a dry run trims nothing.
- Then reads the free disk of the placed machine the settings name, the host drive a virtual disk stands
  on among it, and sends `placed-disk-low` where a disk there stands below the floor (「The placed
  machine」); `--all` reads none.

### `full`

```
bmo-testbed full [--due <HH:MM>]
```

- Starts the daily whole run of the repository in a unit of its own and ends, or with `--due` only
  where none started since that time of day (`docs/suite.md`, 「The daily whole run」). Inside its own
  unit it runs to its end and exits 1 where it failed. Exits 2 where another run holds the lock.

### `deploy`

```
bmo-testbed deploy [--commit <sha>] [--message <why>] [--dry-run] [--no-check] [--before-start <command>] [--force-live] [--first <why> [--by <who>]]
bmo-testbed deploy first <unit> --why <why> [--by <who>]
```

- Puts the commit live (`docs/deploy.md`). `--message` is the reason the deploy is made, written into
  the record and the `deployed` event; the commit's subject stands where none is given. Outside a
  deployment's unit it starts one, prints its name
  and the command that reads its journal, and exits 0; where a deployment of the same repository and commit,
  read as commits, already stands for the deploy root, waiting or under way, it starts none, prints that
  deployment's unit, how long ago it asked and the command that reads its journal, and exits 0, unless it
  is asked with `--before-start`, `--no-check` or `--force-live`; inside it, it runs and exits 0 where the commit
  went live, or was live already when its turn came, 1 where it failed or was rolled back, 2 where a
  deployment of its commit failed in its gate while it waited. Its refusals exit 2 in the caller's own
  process, among them a commit whose newest ended deployment failed in its gate while the main branch still
  stands on it, unless it is asked with `--first` or `--no-check` (`docs/deploy.md`, 「The deployment
  line」).
  Deployments started together each run in their own unit and pass their gate and their build one after
  the other, in the order they asked, through the deployment line under the data root's `locks/`
  (`docs/deploy.md`, 「The deployment line」). A deployment of a repository whose earlier deployment is
  still under way waits, before the line, until that one has ended, so one deployment of a deploy root
  runs at a time.
  `--dry-run` says in the caller's own process what it would do, changes nothing and starts no unit.
  `--force-live` goes on at once past a declared `quiet` command that names work under way or could not
  tell, instead of waiting for it to answer 0 (`docs/deploy.md`, 「What the deploy run does, in order」),
  saying what it answered. A `quiet` that could not tell, left unforced, fails the deployment once it
  repeats the policy's number of asks running.
  `--first` marks the deployment first in the deployment line, with the English reason given: it goes
  ahead of every deployment waiting there, never ahead of the one holding it, and those marked first
  go in the order they were marked (`docs/deploy.md`, 「The deployment line」). `--by` names who marked
  it, the account the verb runs as where none is given.
- `deploy first` raises a deployment already waiting, before the line or in it, to first, by its unit,
  with the English reason `--why` gives and who `--by` names, the account the verb runs as where none is
  given. It prints one line naming the unit, who and why, and exits 0. It refuses with 2 a unit that is
  not a running deployment unit, one the testbed's record already holds, the one holding the line and
  one already marked first.

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
  A step that failed is said on a line of its own, and a run that failed ends with a line naming its event,
  `data-backup-failed` or `restore-drill-failed`, and the event's summary; where the output goes to the
  journal, those lines are written at its error priority (`docs/backup.md`, 「What it is」).
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

### `notice`

```
bmo-testbed notice <event>
```

- Tells the owner's supervisor of an event, taken as a hook takes it (「Events」): the event's name the
  word, its payload on standard input. It is the placed machine's hook where none is named, and any hook
  setting may name it.
- **The event goes to the owner's private room as a quiet instruction,** `POST /instructions` (the
  receiver's `docs/api.md`, 「Instructions」), which queues it for the agent that answers that room. The
  room is the one the receiver's `GET /owner` names as the owner's private room, asked first. The
  instruction is said as a fact: `code`, `testbed:` and the event's name, with the payload's fields
  standing beside it. Its `sort` is `notice`, its `kind` and `summary` the code, and its `publisher`
  `testbed`. It names no `cause`, so it is a room reply that opens the supervisor's turn as soon as it
  is queued. The receiver keeps one pending instruction of a kind in a room, so a newer notice of the
  same event replaces one not yet said.
- **The receiver is found as its own tools find it:** `RECEIVER_ADDRESS` and `RECEIVER_TOKEN` from the
  environment, else from `<data root>/env/receiver.env`, which the receiver writes as it is deployed.
  Each is read out of that file as the machine's hook is read out of its own (「Events」), and nothing
  else of the file. The token goes into the requests' authorization and nowhere else.
- The receiver is given the policy's twenty seconds to answer each request.
- Exits 0 once the receiver answers that the instruction is queued. Exits 1 where the receiver is not
  found, does not answer, refuses, or names no private room for the owner: the reason goes on a line,
  then the event's line as an event with no hook writes it, so the event still stands wherever the
  verb's standard error goes. Exits 2 where the payload is not one JSON object.

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
| `BMO_DATA_DIR` | every verb, the hosted deploy tool | The testbed's data root: logs and the record under `testbed/`, locks under `locks/`, the data backup's and the placement's settings under `env/`. Default `$HOME/.local/share/bmo/data` |
| `TMPDIR` | the turn | Where the lane locks stand. Default `/tmp` |
| `BMO_TESTBED_LOCK` | the turn | Another first wide seat's lock, the further seats and the narrow lane's beside it |
| `BMO_TESTBED_SEATS` | the turn | The wide lane's seats, over the lane policy's; anything that is not a positive whole number means the policy's |
| `BMO_TESTBED_SEAT_FREE_GB` | the turn | The gigabytes of available memory a seat beside a held one needs, over the lane policy's; anything that is not a whole number means the policy's |
| `BMO_TESTBED_LOADED_PERCENT` | the placement, `status --free` | The share of the machine's threads, in percent, at or above which its load figure, the larger of the last-minute and last-five-minute averages, makes it loaded, over the lane policy's; anything that is not a whole number of 0 or more means the policy's |
| `BMO_TESTBED_HELD` | the turn | Set by a holder for its children: its process number. A command finding it holding any seat of its lane runs at once, in the holder's seats |
| `BMO_TESTBED_FRONT` | the turn | `1` stands the caller at the front, unless it is inside a harness's call |
| `BMO_TESTBED_NEXT` | the turn | `1` stands the caller next after the front |
| `BMO_TESTBED_HEARD` | the turn | Set by a holder watching its command for silence: a file of its own. A watch under it touches the file while its own command writes, which the holder counts as its command writing |
| `BMO_TESTBED_TIMES` | the turn | A file each turn taken adds a line to: the command, its lane, its wait and its run |
| `BMO_TEST_PARALLEL` | the turn, the suite | The wide lane's width; set by the turn for a language's runner, to the holder's share |
| `CLAUDE_PID` | the turn, `deploy` | The process of the harness whose call the caller runs in: a caller with one is inside a harness's call, its holder is watched, and `deploy` refuses it |
| `BMO_TESTBED_SCOPE` | the turn, the record | The caller's opaque scope, which a ticket, a holder and a test run carry and the testbed never reads |
| `BMO_TESTBED_TOKEN` | `serve`, `mcp` | The HTTP service's bearer token |
| `BMO_TESTBED_SERVICE` | `mcp` | The HTTP service's address the reading tools call |
| `BMO_TESTBED_PLACEMENT` | `run`, `suite`, `deploy` | The placement endpoint, over the settings and the declaration. Set empty for `deploy`, the testbed's deployment leaves the placed machine's testbed as it stands (`docs/deploy.md`, 「Where a deployment runs」) |
| `BMO_TESTBED_PLACED_STANDING_MS` | every placed run, `placed` | How often a placed run whose output has not come back says it still runs there, and how often the `placed` verb says its command still works while none of its output passes, in milliseconds, over the lane policy's; anything that is not a positive whole number means the policy's |
| `BMO_TESTBED_PLACED` | the turn, `placed` | Set by `placed` for the run it starts: `1`. A run finding it is never placed again |
| `BMO_TESTBED_CHECKOUTS` | `dependents`, `deploy`, `prune`, `check shared-versions`, the hosted deploy tool | The machine's registry of checkouts, over the data root's `testbed/checkouts.toml` |
| `BMO_TESTBED_EVENT_HOOK` | every verb that sends events | The event hook, over the declaration's, and the hook for a verb run outside a repository |
| `RECEIVER_ADDRESS` | `notice` | The receiver's base address, over the one `<data root>/env/receiver.env` names |
| `RECEIVER_TOKEN` | `notice` | The receiver's bearer token, over the one `<data root>/env/receiver.env` names |
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
| `BMO_CARGO_TARGET_CEILING_GB` | `prune` | The shared Cargo build directory's ceiling, in gigabytes of a thousand million bytes, never more than its filesystem can hold with the prune policy's room left free; anything that is not a positive whole number means the policy's, 200 (`docs/deploy.md`, 「The Cargo build directory」) |
| `BMO_CARGO_TARGET_DEPLOYMENT_CEILING_GB` | `prune` | The deployments' checkouts' own ceiling under the shared Cargo build directory, in gigabytes of a thousand million bytes, which the prune holds them to by least recent read; anything that is not a positive whole number means the policy's, 0, which sets none and keeps them whole (`docs/deploy.md`, 「The Cargo build directory」) |
| `XDG_CONFIG_HOME` | `deploy` | Where the user units are installed |

## The declaration

`testbed.toml` at the repository's root, its whole schema in `docs/declaration.md`. In short:

- `testbed`: the major targeted;
- `upstreams`: the repositories this one builds on, by their declared names;
- `[test]` with `[test.database]`: the languages, files, fence and setup of the suite, the sibling
  trees its tests read, and the silence limit of every command run for the repository;
- `[projects]` and `[tools]`: units found by their own files;
- `[check]` with `[[check.step]]`: the quick checks enabled and their inputs, the toolchain the
  workspace's formatting runs on, and the static steps;
- `[build]` with `[[build.step]]`: the release build;
- `[deploy]` with `[[deploy.step]]`, `[deploy.console]` and `[deploy.backup]`: the deploy tree, data root,
  env links, units, health, declared steps, the console screen the deployment probes and backups;
- `[runner]`: the placement endpoint and the variables and files that keep a run here;
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
- **The placed machine's events go to the machine's own hook,** since they are the machine's and no
  repository's: `BMO_TESTBED_EVENT_HOOK` where the caller's environment sets it, else the one
  `<data root>/env/testbed.env` names, the file the timed prune's service reads as its environment
  (`docs/deploy.md`, 「The Cargo build directory」). The testbed reads that one variable out of the file as
  the service manager reads it, the last line assigning it, its value trimmed and a value in quotes
  unquoted, and nothing else of the file. No declaration's hook takes them.
- **Where neither names a hook, the placed machine's events go to the owner's supervisor:** the hook is
  then the running testbed binary's own `notice` (「`notice`」); a program that calls the placement in its
  own process, such as a test harness, has none, and the event is its line. An outage of the placed machine, or its testbed
  left behind the live release, stops every run placed there, and only the owner can act on it. The
  route that tells him is the receiver's instruction queue, the platform's one road for a system notice
  to the owner: it holds the instruction until his supervisor's session can take it, rather than
  handing it to a session that may be at rest, and the supervisor says it in his private room (the
  receiver's `docs/voiced-notices.md`, 「Delivery」). It needs only the receiver's own token, which the
  receiver writes for every process of the machine, so no secret stands in the testbed's code, policy
  or settings. The receiver is named here only as the platform's queue of instructions, by its fixed
  API, and nothing of any repository stands with it.
- **The hook never changes a verb's outcome.** It is waited for up to the policy's time, sixty seconds;
  one still running then has its process group ended, and one that fails or overruns is said on a line
  and the verb goes on. A hook that reaches a peer which is down therefore holds a verb a minute at
  most, and the event is not sent again.
- **A deployment sends its outcome events after its outcome is written.** `deployed`, `deploy-failed`
  and `deploy-rolled-back` are sent once the progress record and the record hold how the deployment
  ended, so a reader of either learns the outcome before any hook runs.
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
  | `cargo-target-over-ceiling` | `bytes` (what the directory holds once the prune's enforcement is done), `ceiling_bytes`, `holders`, each with `path`, `by` (`build-lock` for a profile directory whose build lock a build holds, `deployment` for the directory of a running deployment's checkout or build tree, `program` for a checkout's directory a program built there runs from, `deployment-checkout` for a deployment's checkout, which the directory's ceiling never removes from) and `bytes`. Sent only where the enforcement could not bring the directory under its ceiling |
  | `placed-unreachable` | `machine` (the placed machine's label), `since` (when the outage began, RFC 3339), `command` (the words of the run that told it), `waited_seconds`, `said` (what `ssh` said last). Sent once for each outage, by the first run that waited its bound for the placed machine to answer and ends as not run (「The placed machine」) |
  | `placed-recovered` | `machine`, `since` (when the outage began). Sent once the placed machine answers again after an outage `placed-unreachable` told, by the one process that finds it so |
  | `placed-release-failed` | `machine`, `command`, `live` (the live testbed release's commit), `far` (the commit the testbed there names, or null), `said` (what bringing it came to). Sent once for each live release, by the first run whose placed machine answers but whose testbed could not be brought to that release; that run and every one after it ends as not run. A run that finds the testbed there at the live release clears this, so a failure after it is sent again (「The placed machine」) |
  | `placed-disk-low` | `machine` (the placed machine's label), `floor_bytes`, `disks`, each with `path` (as the settings name it), `source` (the filesystem's source, the drive itself for a WSL drive mount) and `free_bytes` |
  | `disk-trim-failed` | `path` (the build directory the trim was run on), `said` |
  | `full-suite-failed` | `head`, `log`, `step` (the failing setup or static step, or null), `failures`, each with `unit` and `package` |
  | `suite-flaky` | `tests`, each with `test` and `unit`, `units`, `log` |

## The record

- **What the verbs did is written into one record** on the machine, `record/record.sqlite` under the
  data root's `testbed/`, which every verb writes and the HTTP service reads: the test runs, the
  deployments with their steps and log lines, the restarts, and the events of each.
- **A test run** is written by every `run` that is not placed elsewhere, kind `command`, and by every
  `suite`, kind `suite`, a deployment's gate and the daily whole run among them. It is written once
  its turn has come and ended with its counts; a run ending not run from there on ends `not-run`. A change
  check over several checkouts writes one test run of kind `check` for each repository, its checkout, its
  repository's name and its commit, once the set's turn has come. A run placed elsewhere is written by the
  machine it runs on, a change check over several checkouts among them: its test runs stand in the record of
  the machine that ran its steps, each naming its checkout as it stands there, and none is copied to the
  machine that placed it.
- **A test run carries what it ran on, read as its turn comes:** the checkout's `HEAD`, whether the
  checkout's tracked files stood unchanged against it (`git status --porcelain --untracked-files=no`
  answering nothing), and whether it was a run of the whole suite. A deployment's gate reads them to
  leave its suite out (`docs/deploy.md`, 「What the deploy run does, in order」, step 3); a run written
  before the record held them holds null, which never stands for a passed whole suite.
- **A gate's pass** is written by a deployment whose gate passed, on the machine the deployment runs on
  whatever machine the gate ran on: the repository, the commit, the commit the gate's range began at,
  the heads of the repositories laid beside the tree it judged, the toolchains `toolchain` answered
  there, that machine's label, and when. A later deployment's gate reads it to leave the whole gate out
  (`docs/deploy.md`, 「What the deploy run does, in order」, step 3). It is no member: no route serves it
  and no event tells of it.
- **A deployment** is written as the deploy run begins its progress record and changed with it
  (`docs/deploy.md`, 「What a deployment says of itself as it runs」), and every line the deploy run
  says is one of its log lines.
- **A restart** is written by a deployment for every unit it stops or restarts, before it does: the
  unit, the deployment, and one English sentence of why, so the unit reads why it was stopped as it
  starts again.
- **Writing the record never changes a verb's outcome.** A record that cannot be opened or written is
  said on one line of standard error and the verb goes on.
- **Every opener and every writer waits its turn rather than being refused.** The record is kept in SQLite's
  write-ahead-log mode, set as the file is first made, and turning a fresh file into that mode takes it whole:
  SQLite answers busy at once, without waiting, to a connection whose wait could deadlock with another
  doing the same. So whoever opens the record sets it up holding an exclusive lock on the file
  `record.sqlite.lock` beside it, one opener after another, and every write takes the record's write lock as its
  transaction begins, waiting as long as another writer holds it, rather than reading first and being refused
  when it comes to write.
- **What it keeps:** test runs, deployments, their log lines, restarts and gates' passes for the policy's days, and
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
- **Queue:** `name` (`wide` or `narrow`), `width` (integer, the lane's whole width, which a holder of
  every seat runs on), `holders` (a list of queue entries, one for each seat held, in the seats'
  order, the lane's deployment's seat last, so a holder of several seats stands in it once for each),
  `holder` (the first of `holders`,
  or null where no seat is held), `waiting` (a list of queue entries, in the order they will be
  served), `placed` (a list of queue entries, one for each run of this lane this machine placed on
  another machine and has not seen end, in the order they were placed; 「A run placed elsewhere stands
  here too」).
- **Queue entry:** `scope` (string or null, the caller's opaque scope from `BMO_TESTBED_SCOPE`),
  `checkout`, `command` (one line), `pid`, `askedAt`, `heldAt` (null while waiting), `machine`
  (string or null: the label of the machine a placed run runs on, null for an entry of this machine's
  own seats and line), and the booleans `front` and `next`, and `kept`, always false, since no place is
  kept for a waiter that left.
- **Test run:** `id`, `kind` (`suite`, `command` or `check`), `checkout`, `repositoryName` (string or null),
  `commit` (the checkout's `HEAD` as the run began, or null where git could not tell), `whole` (boolean
  or null: true for a suite run of every unit, with no range, no word naming a path and no word for the
  runner; false for any other run), `clean` (boolean or null: whether the checkout's tracked files stood
  unchanged against its `HEAD` as the run began, null where git could not tell), `command` (the words
  run, as one line), `scope` (string or null), `unitName` (the unit it runs in, from `BMO_DEPLOY_UNIT`,
  or null), `pid`, `state`, `passedCount`, `failedCount`, `skippedCount`, `logPath`, `startedAt`,
  `endedAt` (null while it runs). `whole` and `clean` are null for a run written before the record held
  them.
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
  | `queue.changed` | the queue whose holders, line or placed runs changed |
  | `test-run.created` | the test run as it began |
  | `test-run.ended` | the test run as it ended |
  | `deployment.created` | the deployment as it began |
  | `deployment.changed` | the deployment after each change of its record |
  | `restart.created` | the restart |

- **A lane's change is seen by the service itself,** which reads the lanes every second of the
  policy's and writes a `queue.changed` event where a holder, a line or a placed run differs from the
  last reading. A reading whose events could not be written is said on standard error and does not become
  the last reading, so the next reading writes the change again, as the lanes then stand; no change is
  lost to a record that could not be written for a moment.
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
  | `testbed_check` | `bmo-testbed check --json`, in the checkout named, from `since` where named; over several checkouts, each a `--checkout` with its own `since`, where `checkouts` names them | a command run |
  | `testbed_suite` | `bmo-testbed suite --json`, in the checkout named | a command run |

- **A tool that fails** answers a tool error whose text is JSON: `status` where a route answered,
  `code` and `error`. Arguments that are not the tool's shape are `bad-request`; a service nothing
  answers at is an error with no `code`.
- **It sends no notifications,** since the service keeps no subscriptions.

## The hosted deploy tool

- **One tool the tool host runs,** in bmo-pocket's call protocol 1 (bmo-pocket's `docs/mcp-host.md`,
  「Placed tools」): the tool `deploy`, whose one entry `testbed/deploy` is shown to a session whose
  record holds the capability `deploy`, and which puts a commit live for a session `deploy` refuses
  (`docs/deploy.md`, 「The hosted deploy tool」). It serves no resources.
- **Its arguments** are `repository`, `commit` and `why`, strings and required, `dry_run` and
  `attach`, booleans, and `first`, a string; its input schema, beside its manifest, takes no other.
  `attach` starts nothing and answers the outcome of the newest deployment of the commit the testbed's
  record holds. `first` marks the deployment first in the deployment line with that reason, the
  session record's `holderName` as who; where a deployment of the same repository and commit already
  waits, the call raises it and follows it instead of starting another.
- **Its answer** is text. A deployment that went live answers a first line beginning `live:`, naming the
  commit and its subject. A deployment that did not, a refusal and arguments that do not read answer the
  same text marked as an error, `isError`. A dry run answers what the verb said.
- **Its entry declares `ends_at_stop`,** so a call of it holds neither the tool host's stop nor its
  `quiet`. A call the host's stop ends is answered by the host as failed; the deployment goes on, and a
  call with `attach` answers its outcome.
- **The testbed's own deployment places it** from the release's `tools/deploy/.placed/` into the tool
  host's registry, for the service `testbed`.

## Placement

- **Where a heavy run goes is the machine's placement:** the default is local, so a testbed with no
  placement runs everything where it was started. A placement is one of two members: a placed machine
  the settings name, reached over `ssh`; or a placement endpoint asked over HTTP. The endpoint is
  `BMO_TESTBED_PLACEMENT` where set, else the declaration's `[runner] placement`; the placed machine is
  `<data root>/env/placement.toml`. The variable is read first, then the settings file, then the
  declaration.
- **A run started on a placed machine is never placed again:** the remote side runs it inside a held
  turn of its own, and the `placed` verb marks its run with `BMO_TESTBED_PLACED`.
- **A run inside a held turn, or inside a whole run's unit, is never placed.** A deployment's own heavy
  work — its gate and its Cargo release build — goes to the placed machine the settings name
  (`docs/deploy.md`, 「Where a deployment runs」) and nowhere else; anything else a deployment's unit
  runs stays where it is.
- **A run whose environment sets a variable its checkout's declaration names in `[runner]
  local_variables` is never placed,** said on a line naming the variable. A placed run carries none of
  the caller's environment but its place in the line, and what such a variable names, a server on this
  machine's loopback address among them, only this machine reaches. The variables a repository's runs
  read are that repository's to name, so the platform's own addresses every session carries keep
  nothing here.
- **A run started in a checkout holding a file its declaration names in `[runner] local_files` is never
  placed,** said on a line naming the file. The copy carries no file git ignores, and what such a file
  names, a test environment pointing at servers on this machine's loopback among them, only this machine
  reaches.
- **A run started in a folder the bench copy leaves out is never placed,** said on a line naming the
  folder: one git names as ignored in its checkout, or one inside such a folder, which the copy
  (「The placed machine」) does not carry, so the bench would not hold the folder the command runs in. A
  run placed from any other folder runs in the same folder of the bench (`docs/testbed.md`, 「A run on another
  machine」), the `placed` verb's `--dir` and the endpoint's `directory` naming it.
- **The runs that can be placed** are a `run`, a `suite`, the change check and the Cargo verb's `test`,
  `check` and `clippy` (`docs/testbed.md`, 「A run on another machine」, which also names the runs that
  stay here and why).
- **The placed machine's settings put the machines in order** (「The placed machine」, `order`): the
  placed machine and this machine, `local`. A run goes to the first in the order that takes a run now, a
  seat of its lane free, nobody waiting for one and the machine not loaded (`docs/testbed.md`, 「A run on
  another machine」), and falls to the next only where the machine takes none. This machine's seats are
  read from its lanes and its load from its kernel; the placed machine is asked its `status --free`, of the
  run's lane, before its bench is made ready; an answer that does not say the machine takes one counts as one
  saying every seat is held or waited for, the line it gave saying why passed on. A machine full of seats takes
  the run only where it stands last in the order, and then waits for it in its line; a machine loaded past its
  share takes none wherever it stands. Where no machine of the order takes the run, it runs where it was
  started. A placed machine that cannot be reached is waited for, and one whose testbed is not
  this machine's live testbed release is brought to it, never passed over (「The placed machine」); where neither can be had,
  the run ends as not run, exit code 75, and sends an event (「Events」). The placement endpoint chooses
  for itself and is told nothing of the order.
- **A change check over several checkouts is placed as one set** wherever the settings name a placed
  machine that takes it. It walks the placed machine's `combined_order` (「The placed machine」), whose default
  is the placed machine alone, so while that machine is calm it waits in its line however long that takes, and
  the set goes on to the machines that order names after it as any run walks `order`, a loaded machine turning
  it away as any run. Where the
  machine cannot be reached within the wait, or its testbed cannot be brought to the live release, it ends
  as not run, exit code 75, on the line naming the machine, as any placed run does. It runs here only where
  `combined_order` names `local` and the walk comes to it.
- **A testbed whose settings name no placed machine runs the set here,** in this machine's own turn, as the
  change check over every checkout by its path, the same check a placed machine runs; so does a set kept
  here by a variable or a file (below) where no placed machine is named.
- **Every other placement refuses the set,** exit code 1, on a line saying why, and runs nothing: the
  placement endpoint, whose request carries one checkout; and, where the settings name a placed machine
  whose `combined_order` does not name `local`, a run whose environment sets a variable one of the
  checkouts' declarations names in `[runner] local_variables`, or a checkout holding a file its
  declaration names in `[runner] local_files`, the line naming the variable or the file. Where
  `combined_order` names `local`, such a set stays here as such a run does.
- **The set is laid in one bench and run there by one `ssh` command:** every checkout made ready at its own
  place under the bench root, its commit with its open files over it, and every tree any of them reads by a
  path laid beside them once; a tree whose place is one of the set's checkouts is that checkout and is not
  copied over it. A checkout whose declaration names `[test].setup` steps has them run in its own bench
  through the `placed` verb's `--setup`; the set then runs through the `placed` verb in the first
  checkout's bench, as the testbed's change check there over every checkout by its path from that bench,
  knowing every bench and tree laid. It stands here as one placed run, under the first checkout.
- **In the crates the placement is a trait** in `core` with two methods. The first is asked with the run's
  words and lane and answers here or elsewhere with an exit code; the local placement, always here, is the
  default, the placement kept here by a variable says so and answers here, and the placed machine, which
  walks its order, and the HTTP placement in `cli` are members of it. The second is asked with a change
  check's set of checkouts, and its default refuses the set on a line saying the placement carries one
  checkout; the local placement keeps it here, the placement kept here by a variable keeps it here where
  it was made knowing no placed machine is named or that `combined_order` names `local` and refuses it
  naming the variable otherwise, and the placed machine walks `combined_order`.

### The placed machine

- **The settings** are one TOML file, `<data root>/env/placement.toml`:

  | Key | Type | Default | Meaning |
  |---|---|---|---|
  | `machine` | string | the destination | The label the lines name the machine by |
  | `destination` | string | none: placement paused | The machine as this machine's `ssh` reaches it. Where none is named, placement is paused (below) |
  | `bench` | string | `~/.local/share/bmo/bench` | The runs' bench root there: every tree a run copies over stands under it at the path it has under this machine's home |
  | `deploy_bench` | string | `~/.local/share/bmo/deploy-bench` | The deployments' bench root there: every tree a deployment copies over stands under the folder of its repository's name in it, at the path it has under this machine's home, the bench itself at its repository's one place (「A deployment's bench stands apart」) |
  | `ssh` | list of strings | `["ssh"]` | The `ssh` program and its options |
  | `testbed` | string | `~/.local/share/bmo-testbed/deploy/bin/bmo-testbed` | The testbed binary there, its `built-from` beside it; the testbed's deployment copies its release's binary of the same name and its `built-from` onto them |
  | `cargo_target` | string | `~/.local/share/bmo/cargo-target` | That machine's shared Cargo build directory; a deployment's build is read back from the bench's own directory under it, the one that machine's Cargo verb builds in (`docs/deploy.md`, 「The Cargo build directory」) |
  | `disk_free_gigabytes` | number | 50, the prune policy's | The free space, in gigabytes of a thousand million bytes, below which a disk of that machine sends `placed-disk-low` from this machine's prune (`docs/deploy.md`, 「The Cargo build directory」). Not negative |
  | `disk_paths` | list of strings | none | Paths on that machine whose disks' free space the prune reads beside `cargo_target`'s. A machine on a virtual disk names the mount of the host drive the disk file stands on, `/mnt/c` for a WSL machine whose disk file is on `C:`, since the virtual disk's own free space says nothing of that drive |
  | `order` | list of strings | the placed machine, then `local`; `local` alone where placement is paused | The machines a run that can be placed goes to, first choice first (「Placement」): the placed machine by its label, `machine` or else `destination`, and this machine as `local`. Each at most once and none other; an empty list does not read. A deployment's gate and release build go to the placed machine whatever it says |
  | `combined_order` | list of strings | the placed machine alone; `local` alone where placement is paused | The machines a change check over several checkouts goes to, first choice first, read as `order` is (「Placement」). Left at its default the set never runs on this machine and waits in the placed machine's line; naming `local` lets the set fall to this machine as `order` lets a run |

  A `~` opening a path there is that machine's home. A file that does not read is no placement, said
  on a line.
- **A file that names no `destination` pauses placement.** It reads as no placed machine: every run, a
  deployment's gate and its release build run on this machine, as where no file stands, and nothing is said
  on any run's line. The rest of the file is read all the same, so a machine taken out for a while keeps its
  settings beside the commented-out lines that named it: a key that does not read still makes the file not
  read, and `order` and `combined_order` may name `local` alone, since no placed machine stands to be named.
  `status` says on a line that placement is paused, naming the file, and its `--json` answers `placement`
  `paused` (「`status`」).
- **The testbed there is this machine's live testbed release.** A testbed running from a release reads,
  before it makes a bench ready, the commit the `built-from` file beside that machine's `testbed` binary
  names, and compares it with the commit of the live release: the release the deploy tree its own
  release stands beside names at that moment, which a process started from an older release reads as
  the newer one. Where they differ, or no such file reads there, it first brings the testbed there to
  the live release, as the testbed's own deployment brings it to a release it puts live (below), with the
  live release as the source, and says so on a line naming the machine and the commit. One process of
  this machine compares and brings at a time, under the lock `placed-testbed.lock` under the data root's
  `locks/`, reading the live release again once it holds the lock. Where that does not take while the
  machine answers, a deployment fails its stage, and a `run`, a suite, a change check or a Cargo verb
  ends as not run, exit code 75, and sends `placed-release-failed` where no run sent it for that live
  release (below), each naming both commits and what the machine answered; nothing is run here in its
  stead. A deployment whose caller sets
  `BMO_TESTBED_PLACEMENT` empty brings nothing and fails its stage where the releases differ. A testbed
  running from no release, such as a checkout's own build, compares nothing.
- **The testbed's own deployment brings that machine's testbed to its release.** Once the release is
  live and nothing can roll it back any more, after the steps after the health, the releases' prune
  and the units laid there, a deployment whose release's `bin/` holds a file of the name the settings'
  `testbed` ends in copies that binary onto `testbed` there and runs it there once, asking its help, then
  copies the release's `bin/built-from` beside it, reads that `built-from` back and checks it names the
  commit put live, all under the same lock (`docs/deploy.md`, 「Where a deployment runs」, and step 16 of
  「What the deploy run does, in order」). It reads `destination`, `ssh`, `machine` and
  `testbed`. Its failure is a warning, never a failed deployment: a machine that cannot be reached, a copy
  or a read that fails, another commit read back, or a testbed that does not run there is said on a line
  naming the machine and what it answered, and the deployment stays live. Nothing is copied where the
  settings name no machine or the caller sets `BMO_TESTBED_PLACEMENT` empty.
- **The caller makes the bench ready** before it runs: where the checkout's place under the bench root
  holds no repository it clones the checkout's reachable remote there, or makes an empty repository where
  the checkout has none or the clone fails there, the push then carrying the history the bench lacks;
  the reachable remote is the checkout's `origin`, and where that names a repository on this machine, as
  a support checkout cloned from its main checkout does, the `origin` that repository names in turn,
  followed until one names a repository elsewhere, and none where the chain ends on this machine; and
  where the place stands as a folder already — a tree an earlier run copied there beside another
  checkout — it makes the repository in that folder; it pushes the commit there as the branch `bmo-bench` and checks it out detached;
  for a run it copies its open files over with `rsync`, leaving out the `.git` directory and each path
  git names as ignored there by its own rules, a `.gitignore`'s `!` lines among them, so what git takes
  back is copied and what is deleted here is deleted there, and keeping each path the bench's own git
  names as ignored in the bench as it stands there. The copy goes by content, not by time: a file goes over
  only where its content differs from the bench's, and it stands there with the moment of the copy rather
  than its modification time here, while a file whose content the bench already holds keeps the time it has
  there. A build there reads a source by its modification time against what it built before, so a file
  whose time here is older than that build is still one it builds, a file the bench's checkout has just
  written is never set back to an older time, and an unchanged file builds nothing again; and for a deployment it copies none and takes every untracked file away, since the release
  is the commit alone; and it lays
  every tree outside the repository the checkout's workspace reads by a path, for a deployment the
  workspace at the deployed commit, read as a deployment here reads it, the base libraries and
  the sibling crates among them, each tree those name in turn followed until none is new
  (`docs/deploy.md`, 「The releases」), each tree the declaration's `[test] sibling_trees` names by a
  path from the checkout, and the checkout holding the shared versions the declaration names, found
  through this machine's registry, each at its own place under the bench root, so a path a manifest names
  resolves there as it does here, the quick checks find what they read, and a test opening a file in a
  neighbouring checkout reads what it reads here rather than whatever an earlier run left at that place. A
  sibling tree that does not stand here fails the step, naming it. For a run it copies
  each tree as it stands, the same way as the open files but for what the copy there keeps, which is
  what git ignores in the tree here, since the copy holds no repository; a tree no git repository holds
  fails the step, since no ignore rules say what to leave out of it. For a deployment it lays no working tree: the
  repository holding each tree is made ready at its place as the bench is, from its own reachable
  remote, pushed the head of the main branch its declaration names (`docs/declaration.md`, 「The main branch's
  head」) as `bmo-bench`, checked out detached
  and its untracked files taken away, since a deployment builds committed trees only (`docs/deploy.md`,
  「The releases」); a tree no repository holds, or whose main branch does not stand, fails the step. The copy of
  the checkout holding the shared versions leaves their file out, and the caller writes it there instead
  as that checkout's main branch holds it, whole or not at all, so no copy, its own or another run's
  copying the same tree, puts the working tree's file there; a placed run reads it as it stands
  (`docs/checks.md`, 「The quick checks」, `shared-versions`). The bench and
  every tree laid beside it are the checkouts the run there knows (「`placed`」), but for a checkout that is
  itself one of the repository holding the shared versions, its declaration giving that repository's name:
  the run knows that repository once, by the copy of the checkout found through this machine's registry,
  and the bench is among its checkouts only where it is that copy. A worker's copy of that repository
  therefore reads the file as the primary checkout's main branch holds it, as it does here, and is never a
  second checkout declaring the name. A step that fails ends the run as
  not run, exit code 75, with a line naming the step and what it said.
- **A deployment's bench stands apart** from every run's and from every other repository's deployment:
  its checkout and each tree laid beside it stand under the folder of the deployed repository's name
  in `deploy_bench`, at the paths they have under this machine's home, made ready the same way. A run
  copies a sibling repository's tree as it stands into its own place under `bench`, and a deployment of
  another repository lays its committed trees into its own folder, so nothing another run copies is written into a tree a
  deployment's gate or Cargo release build reads while it reads it, and the commit a deployment checks
  out in its bench, with the untracked files it takes away, never reaches a tree a run reads.
- **Each repository's deployments keep one bench there, at one path,** whichever of the repository's
  checkouts a deployment is taken from, so the Cargo build directory that machine's testbed names after
  the bench stays warm from one deployment to the next. The bench stands where the repository's main
  checkout, the work tree its common git directory belongs to, would stand beside the checkout: in the
  folder holding the checkout, under the main checkout's folder name. A worktree standing beside its main
  checkout therefore benches at the main checkout's place, and every path a manifest names upward from
  the checkout resolves there as it does here. A checkout whose common git directory is no work tree's
  `.git` benches under its own folder name.
- **A bench another checkout of the repository left there goes.** Once the bench is ready and every tree
  the deployed repository's manifests name by a path has been laid beside it, each repository standing
  beside it in the same folder there whose root commits are the bench's own is removed: a bench an
  earlier deployment made at another checkout's place, which no deployment reads any more. The sweep
  never takes a tree laid for this deployment, nor a folder holding one deeper down: a sibling repository
  whose history was split from the deployed repository's shares its root commits, so the roots alone do
  not tell another checkout's leftover from a path dependency, and it is the laid list that keeps them
  apart. Every laid tree stands on the bench before any step runs there, and where one does not once the
  bench is made ready, the bench is at fault and not the commit: the making fails naming the missing
  trees, no step runs, and the deployment reports the bench rather than a check the commit never failed.
  A removal that fails is said on a line and fails nothing. The directory that machine's build root kept
  for a removed bench is then the directory of a checkout that no longer exists, which its prune takes
  away (`docs/deploy.md`, 「The Cargo build directory」).
- **The run is then one `ssh` command:** the testbed binary there, in the bench, asked the `placed`
  verb with the words. Its standard error is this run's standard error, its standard output its one
  result line, and its exit code this run's exit code. The caller passes both on as they come, but for
  the line saying the run's turn there came (「A run placed elsewhere stands here too」), touching the file
  its own holder handed it in `BMO_TESTBED_HEARD` while they do; ending a silent command is the `placed`
  verb's there.
- **A failing log kept there comes back.** Each line of either stream that is a change check's `failing log
  kept:` line, whatever repository name opens it, names a file there; once the run has ended
  the caller reads each such file with `cat` over the same `ssh`, writes it under this machine's data root
  at `testbed/placed/<machine>/<file name>`, `machine` the settings' label, and says on standard error
  `testbed: the failing log kept on <machine> is brought here: <path>`. A file that cannot be read there or
  written here is said on standard error with what was answered, and the run's exit code stays the one it
  ended with there.
- **A connection that cannot be made is waited for:** the caller asks the machine again every
  `unreached_pause_seconds` of the lanes policy (30), saying on a line each time how long it has waited
  and what `ssh` said, and goes there once it answers; it takes no seat here meanwhile. Once
  `unreached_wait_minutes` (20) have passed, the run ends as not run, exit code 75, on a line naming the
  machine and what it said last. A connection that breaks once the run has begun ends the run as not run,
  exit code 75.
- **An outage is remembered under the data root's `testbed/placed-outage/`,** one file for the machine,
  made by the first process of this machine that finds it not answering, a run's or a deployment's, and
  holding when that was. The first run that gives up on it sends `placed-unreachable` and marks the outage
  told beside it, so the runs that give up after it send nothing more. The first process that finds the
  machine answering again takes the outage away, and sends `placed-recovered` where it had been told; an
  outage no run gave up on ends untold. Each mark is made only where it does not stand and taken away
  whole, so of the processes racing over one, one alone sends each event.
- **A testbed there that could not be brought to the live release is remembered too,** under the data
  root's `testbed/placed-release-failed/`: one file for the machine and the live release's commit, made
  by the first run that cannot bring it there. That run sends `placed-release-failed`; the runs after it
  that fail on the same release send nothing more. A run that finds the testbed there at the live
  release takes the machine's files away, so a later failure is told again.

### The placement endpoint

- **Before asking for its turn, a run that can be placed posts one JSON object to the endpoint:**

  | Field | Type | Meaning |
  |---|---|---|
  | `checkout` | string | The caller's checkout |
  | `remote` | string | The checkout's reachable remote (「The placed machine」), empty where it has none |
  | `commit` | string | The checkout's `HEAD` |
  | `command` | list of strings | The command's words |
  | `directory` | string | The folder the command runs in, from the checkout's top with `/` between its parts; empty at the top |
  | `lane` | string | `wide` or `narrow` |
  | `front`, `next` | booleans | Where the caller stands |

- **The answer** is one JSON object. Its `place` is `local`, and the command runs here; or `remote`,
  with `machine`, the placed machine's label, `bench`, where the caller's copy of its checkout stands
  there, and `run`, the address of the run there:

  | Field of `bench` | Type | Meaning |
  |---|---|---|
  | `destination` | string | The machine as the caller's `ssh` reaches it |
  | `path` | string | The bench directory on that machine |
  | `ssh` | list of strings, optional | The `ssh` program and its options; `["ssh"]` where absent |

- **The caller makes the bench ready** the way it does for a placed machine, at the path the answer
  names, its open files copied over.
- **The run address is an event stream,** asked once the bench is ready, which runs the bench's
  declared setup at the bench's top and the command in the folder `directory` names, in that machine's
  own turn. Each `output` event's data is one line of the run's output, which the caller writes to its
  standard error; one `result` event's data is one JSON object with `line`, the testbed's result line,
  which the caller writes to its standard output, and `code`, the exit code it ends with.
- **Anything else is local:** one whose connection is refused or breaks, an error, or an answer that
  does not read. The caller waits for the endpoint's answer however long it takes. The caller says so
  on a line and runs here.
- **A stream that ends with no `result`** ends the run as not run, exit code 75.
- **A deployment never asks the endpoint:** its gate and its build go to a placed machine alone.

### A run placed elsewhere stands here too

- **While a placed `run`, `suite`, `check` or Cargo verb lasts, it stands on the machine that placed it,**
  as a queue entry in the `placed` list of the lane it asked for (「Members」), so `status` and
  `GET /queues` name it beside the lane's own holders and line, and a reader ties it to its caller by
  checkout as it does an entry of this machine's own. The entry carries the caller's scope, its
  checkout (the top of the repository it was started in, as a run here names it), its command, the
  process that placed it, when it was placed (`askedAt`), the placed machine's label (`machine`, the
  label the endpoint answered for a run it placed), and its front and next marks.
- **It stands from the moment the run is placed,** its bench made ready meanwhile, until the run
  ends there, is dropped or does not go; an entry whose process is gone is nobody's and is left out.
- **`heldAt` is null while the run waits there,** its bench made ready, its setup run and its wait in
  that machine's line, and is the moment its first turn there came once that machine says so. A testbed
  run with `BMO_TESTBED_PLACED` set writes one line on standard error each time a turn it takes comes,
  `testbed: placed run's turn came`; the placing machine reads that line off the run's output, `ssh`'s
  standard error or the stream's `output` events, and does not pass it on. A run whose far side writes
  no such line stands waiting until it ends.
- **It takes no seat here:** whether a seat here is free, for `status --free` and for a run walking the
  order (「Placement」), is read from the lane's seats and line and the machine's load alone.
- **A deployment's gate and release build on the placed machine stand as the deployment's stages**
  (「The record」), never in a lane's `placed` list.
- **The entries stand beside the lane's lock,** one file for each run in a directory named for the lock
  with `.placed` after it, so every reader of the lanes reads them the way it reads the line.

## The crates

- **Each crate's package is named for its folder** after `bmo-testbed-`: `bmo-testbed-core`,
  `bmo-testbed-scratch-database` and so on. A hosted tool's crate stands under `tools/` and is named
  for its tool with `-tool` after it, as its executable is: `deploy-tool`.
- **`core`:** the lanes, the turn, the queue, the close line, the run and its result, the languages,
  memory limits, the placement, the declaration reader and the policy loader. Its public types are the
  ones the status fields above serialise: the lane, the queue, the entry, the close line and its entry;
  the turn's ask and the turn held; the run's result; the language type every language is a member of.
  The close line of a repository stands where `core` names it from a data root and any directory of any
  clone or worktree of it, so a consumer holding the line and `status` reading it meet in one file.
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
  makes (`docs/suite.md`, 「The test database」): a scratch database, migrated or blank, dropped with
  its guard and failing its test where it cannot be, its address and connection, and the test server's
  address; a consumer's crate may take it as a
  dev-dependency.
- **`testkit`:** the TypeScript package `@bmo-testbed/testkit`, which a consumer may take as a
  dev-dependency (`docs/suite.md`, 「The test home」):
  - `test-stand-ins` answers the colours question on loading, and stands in for a module the consumer
    names, refusing whatever is asked of it; it is preloaded first.
  - `test-home` sets up the file's home from the consumer's ask, the variables to empty and remove, the
    programs to stand in for and the directories to name, and answers the home and its `bin`;
    the consumer's own preload calls it once.
  - `scratch` hands a test a directory taken away as its file ends; `ids` holds the fixed ids.
- **`tools/deploy`:** `deploy-tool`, the hosted deploy tool (「The hosted deploy tool」): its call
  read, the checkout found in the registry, the verb run and its unit waited out, and the outcome answered.
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
