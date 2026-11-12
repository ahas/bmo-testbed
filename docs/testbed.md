# The testbed: a machine's heavy commands in line

## What it is

- **The queue every heavy test and build command on a machine stands in,** whatever harness runs it and
  whatever repository the harness works in: a whole suite, a Cargo build or test, a package script that
  tests, checks or builds, a browser run.
- **Two lanes** (「The lanes」): the wide lane, a few seats sharing the machine's width, and the
  narrow lane beside it, one seat on a quarter of it. A command takes one seat and runs on its share
  of the lane's width; a deployment takes every seat of the wide lane and runs on all of it. A further
  seat is taken only while the machine has memory to spare (「Memory」), so commands side by side never
  run it out of memory and into swap, where a sound change fails its check for a run it never saw.
- **A seat's holder runs on its share of the threads.** Each language's runner is handed the share
  (「The width」), so commands that use a few threads each run side by side instead of one after the
  other.
- **A deployment stands at the front** (「The front of the line」), so its wait is the rest of what
  each lane's holders are running, never the line behind it.
- **The close holding its repository's close line stands next** (「Next after the front」), behind a
  deployment and ahead of every other caller.
- **Only what needs the machine stands in line.** A light command runs where it stands, beside any
  holder (「What goes through it」).
- **Callers still work side by side.** Only their heavy commands queue; editing, reading and light checks
  go on while another harness's tests run.

```mermaid
flowchart TD
  A[a heavy command] --> B{inside a held turn?}
  B -- yes --> R[runs at once]
  B -- no --> P{placed elsewhere?}
  P -- yes --> E[runs on another machine, result comes back]
  P -- no --> Q[stands a ticket in its lane]
  Q --> F{a deployment?}
  F -- yes --> FT[ticket at the front]
  F -- no --> N{a close holding its line?}
  N -- yes --> NT[ticket next after the front]
  N -- no --> OT[ticket at the back]
  FT --> W{turn come?}
  NT --> W
  OT --> W
  W -- yes --> H[holds a seat, or every seat for a deployment]
  H --> R
  W -- harness gone --> K[ends: not run]
  W -- not yet --> W
  R --> V[runs to its end, or is ended once silent too long: one result line, log kept]
```

## What goes through it

- **Taken by the testbed's own verbs with no word from the caller:** a run of the suite
  (`docs/suite.md`), the Cargo verb where its Cargo command compiles, and a deployment's build steps
  (`docs/deploy.md`). Formatting and Cargo's reading verbs run where they stand.
- **Never through it: the prune of the shared Cargo build directory.** It holds Cargo's own build
  lock, standing at the front of that lock's line, and does no work the machine's threads are wanted
  for; behind the wide lane's queue it would wait out every heavy command while the disk fills
  (`docs/deploy.md`, 「The Cargo build directory」).
- **The narrow lane** is for a heavy command that takes gigabytes but only a few threads: a front-end
  type check or production build. A declared check step names the narrow lane for itself
  (`docs/declaration.md`).
- **Any other heavy command is run through the testbed by its caller,** in the wide lane, or in the
  narrow one with `--narrow`: the `run` verb (`docs/boundary.md`).
- **Which commands are heavy** is each language's own answer (「Languages」): a build or test run of
  Cargo, a `bun test` over more than one file, a directory, a name that may match several or a
  browser test, a package script that tests, checks or builds, `node --test`, Vitest, Jest and
  Playwright.
- **What is not heavy:** a `bun test` of one named test file that opens no browser, run once or again
  and again with `--rerun-each`, a type check, the quick checks, a format check, and Cargo's reading
  verbs. Queuing a one-second check behind a three-minute suite costs a harness more than the memory it
  saves.
- **A close holds a seat of the wide lane through each part of its check.** It stands in its
  repository's close line (「The close line」), and inside it holds one seat through its static check
  and again through its suite, standing next after the front; a deployment waiting at the front goes
  in between the two parts.

## A shell hook sends a harness's heavy command to it

- **A harness's shell hook asks the `classify` verb** whether a command line holds a heavy command
  (`docs/boundary.md`). The line is read as the shell splits it, by a shell-language parser: a
  variable the line itself assigned is read as its value, and text run as a command (a string handed
  to `eval`, `source` or a shell's `-c`, a command substitution in the command position) is read as a
  command too. A line the parser cannot read is no command at all.
- **A heavy command started outside the testbed is refused by the hook,** and the answer names the
  line to run instead: the same command after the testbed's `run` and `--`.
- **What is let through:** a light command no language claims, a run of the testbed itself, and a
  command the hook's own consumer names as one that takes the turn itself (`--takes-turn`).

## The lanes

- **The wide lane** is the machine's whole width, shared by its seats: `seats` in the lane policy,
  three, and `BMO_TESTBED_SEATS` names another number for a machine or a test. A command takes one
  seat; a deployment and a check held whole take every seat.
- **The narrow lane** runs beside it on a quarter of the width, one seat, one command at a time.
- **Why seats:** most heavy commands are a check or a build of one repository that keeps one or two
  threads busy for seconds, so holding the whole machine for each leaves most threads idle while a
  line of the same waits for minutes. A few seats side by side keep the threads busy; the memory gate
  (「Memory」) keeps them from running the machine out of memory.
- **Why a narrow lane beside it:** a front-end build keeps two or three threads busy for a minute and
  takes three to six gigabytes, so it stands apart, one at a time, out of the wide lane's line.
- **Each lane has one queue,** in the order asked, with its own front; each seat is a lock of its own
  with its own holder.

## The turn

- **A seat is one lock in the machine's temporary directory.** The wide lane's first seat is
  `bmo-testbed.lock`, its further seats `bmo-testbed-seat2.lock` and on beside it, and the narrow
  lane's one seat `bmo-testbed-narrow.lock`. `BMO_TESTBED_LOCK` names another first wide seat, the
  rest beside it, which is how a test holds a turn of its own. A taker of the first seat's lock alone,
  outside the seats, never marks its ticket seated, so everyone behind it waits until it lets go.
- **The lane's one queue hands its seats out in the order asked.** Only the first ticket not yet
  seated may take a seat: a ticket that has taken its seat is marked seated and stands ahead of
  nobody. The first unseated ticket takes any free seat, where the memory gate lets it (「Memory」).
- **A taker of every seat** (「The front of the line」, 「A check held whole」) standing first takes the
  seats as they free and keeps them until it holds all, then runs. Everyone behind it waits, so it is
  never starved.
- **Each waiter's ticket says who it is:** the caller's opaque scope where `BMO_TESTBED_SCOPE` names
  one, its checkout, the command, its process and when it asked.
- **The holder writes a record beside each seat it holds** naming itself, its process and when the
  turn came, and takes them away when it lets go. A record whose process is gone is read as nobody's.
- **A command started inside a held turn runs at once.** The holder hands its children
  `BMO_TESTBED_HELD`, its own process number, and a command that finds that number holding any seat
  of its lane takes the turn as its own, in the holder's seats: a deployment's suite, a build inside a
  static check and a package script's inner `cargo test` never wait behind the turn their own caller
  holds.

## The front of the line

- **A deployment stands at the front of every lane,** it and everything it starts. Its ticket stands
  ahead of every other ticket, so it takes the lane the moment its holders let go; deployments among
  themselves keep the order they asked in.
- **It takes every seat of the lane,** so a deployment's step runs on the lane's whole width.
- **The holders are never cut.** Standing at the front waits for each seat all the same.
- **A deployment says so through `BMO_TESTBED_FRONT`,** set by the deployment's verbs for themselves;
  a caller inside a harness's call is never stood at the front, whatever it sets.

## Next after the front

- **A caller standing next stands behind every deployment and ahead of every other ticket:** the
  close holding its repository's close line, its hold on the wide lane and every heavy command it
  starts. Callers standing next keep among themselves the order they asked in.
- **It takes one seat,** as an ordinary caller does: closes of several repositories check side by
  side, and none holds the whole machine through its check.
- **The holders are never cut,** as for the front.
- **A close says so through `BMO_TESTBED_NEXT`,** set for its own commands once its turn in the close
  line has come. Unlike the front, a caller inside a harness's call may stand next, since a close runs in
  one.
- **A caller standing next waits its turn out** (「Waiting never fails a command」).

## The close line

- **One line per repository on a machine,** every close of that repository standing in it, the first
  to ask going first. Its lock stands under the data root's locks, keyed by the repository's common
  git directory, so every worktree of one repository shares it.
- **One close checks at a time because a check is of what lands.** In the line each close rebases onto
  the main the close before it landed, so its check runs once, on the commit that lands.
- **The deployment of that commit takes the close's suite as its own.** The close's whole suite is a
  test run in the record of the machine it ran on, with the commit and whether the tree was clean as it
  began; where it passed, the deployment's gate runs its static check and leaves its suite out
  (`docs/deploy.md`, 「What the deploy run does, in order」, step 3), so the suite runs once per commit.
- **A close waits its turn out** while its harness lives, and leaves the line once its harness is gone.
- **It holds a seat of the wide lane through each part of its check** while it holds the line, and
  lets the seat go while it rebases, lands and writes down. Between the two parts a deployment waiting
  at the front goes first.
- **While it waits it says where it stands** every thirty seconds: how many closes stand ahead and
  which one holds the line.
- What a close does inside its turn is its consumer's; the testbed supplies the line, the hold on the
  wide lane and the status.

## A check held whole

- **A deployment's check takes every lane once and holds them to its end:** from its static check's
  first step to the end of its suite, or of the static check where its suite is left out, so no other
  heavy command comes in between its steps. A taker of every lane together takes every seat of each.
- **Where its suite is placed on another machine, it lets the lanes go first,** since that machine's
  own testbed has the turn there.
- **A check that fails lets them go at once.**

## Waiting never fails a command

- **Every caller waits for its turn however long the line takes:** a caller inside a harness's call, a
  deployment, the daily whole run and a person's shell alike. A caller is inside a harness's call where
  `CLAUDE_PID` names the harness's process.
- **A waiter whose harness is gone leaves the line,** since nobody is left to run what it waits for. Its
  command does not run, and says so: a line naming its place and the holders, a verdict of not run with
  nothing tested and nothing failed, and exit code 75. That is the one way a waiter leaves the line without
  its turn.
- **While it waits it says, every thirty seconds, its place in the line and who holds the lane's
  seats,** so a reader sees it is queued and not stuck.

## A holder that dies lets go

- **A holder process that dies lets its seats go with it:** each seat's lock goes the moment the
  process holding it ends, and the next in line takes it.
- **A harness that dies holding the turn lets it go too.** The holder of a harness's turn watches the harness's
  process named in `CLAUDE_PID`; once that process is gone the watch kills the holder and every process
  under it at once, since nobody is left to read what they would answer, and its seats go with them.

## A holder whose command falls silent lets go

- **A command holding a lane's turn that writes nothing for ten minutes is ended as hung,** and its
  seats pass on. Its length alone never cuts it: a command that keeps writing runs until it ends,
  however long that takes, and a person who wants it gone stops it.
- **The holder watches where its command's output goes,** every three seconds: the log of a `run`, the
  files a check step's output is taken into, the pipes a suite's unit or a deployment's heavy step is read
  through, and the files the Cargo verb's own standard output and error stand in. Once none of them has
  grown for the limit, the command is hung.
- **A command under it that is watched in turn speaks for it.** The holder hands its command
  `BMO_TESTBED_HEARD`, a file of its own; a watch under it, such as a suite's over a unit whose output it
  holds back until the unit ends, touches that file while its own command writes, and the holder counts the
  touch as its command writing.
- **A hung command is ended whole:** its process group where it heads one, and every process under it, are
  sent a TERM, and whatever still stands ten seconds later a KILL.
- **It says so in one line,** `testbed:` first, naming the command, where its output went and the limit,
  and ends as failed with exit code 124, never as passed: a `run` on that code, a check's step and a suite's
  unit as failed with it.
- **A person's command whose output is a terminal or a pipe is not watched:** the Cargo verb whose
  standard output or error is a screen, another program or anything but a file runs until it ends, since
  nothing the testbed reads says whether it still writes there.
- The limit, the pause before the KILL and how often the holder looks stand in the lane policy.

## Memory

- **A seat beside a held one is taken only while the machine has memory to spare:** the first
  unseated ticket takes a free seat where the memory available, as the kernel's `MemAvailable` says
  it, is at least `seat_free_gigabytes` in the lane policy, four, or where no seat of the lane is held
  at all, so a lone command always runs. `BMO_TESTBED_SEAT_FREE_GB` names another amount for a machine
  or a test. A taker of every seat runs alone once it holds them, so the gate does not stand in its way.
- **A command the kernel killed for want of memory runs again,** at most four runs in all: before each
  further run the testbed waits at least thirty seconds, then until six gigabytes are free, however long
  that takes, saying every ten minutes that it still waits. The Cargo
  verb and a declared check step that asks for it do so (`docs/declaration.md`); a plain `run` does
  not. A command ended as hung is not run again, since it would hang again.

## The width

- **The wide lane's width is the machine's thread count.** `BMO_TEST_PARALLEL` sets another;
  anything that is not a positive whole number means the default.
- **The narrow lane's width is a quarter of it,** and never less than one.
- **A holder runs on its share of the lane's width:** the width divided by the lane's seats, never
  less than one, for each seat it holds, and the whole width where it holds every seat. A command
  inside a held turn runs on its holder's share.
- **Each language hands the share to its runner its own way** (「Languages」): a `bun test` worker
  count, Cargo's jobs, Node's test concurrency, Vitest's and Jest's workers. A command that already
  names its own number keeps it. The Cargo verb hands Cargo its share as its jobs where neither its
  words nor its environment name them, and a whole run of the suite sizes its sweep by its share.
- **Browsers run narrower:** Playwright is handed a quarter of the width, since every worker drives a
  browser of several hundred megabytes.

## The result

- **A run ends on one line,** opening with `testbed:`: whether it passed, the language that read it,
  how many tests passed, failed and were skipped, the seconds it ran, the seconds it waited for its
  turn, and the log. A command that is not a test run says its exit code instead of counts.
- **Under a failure it prints what failed,** read out of the output by the language that ran it: the
  failing tests, or a compiler's errors, at most sixty lines.
- **The whole output goes to the log,** under `testbed/` in the data root, one file per run, never to
  the caller's screen. Logs older than a week go when the next run starts.
- **Every heavy command says how long it waited and how long it ran.**
- **A caller that wants them written down names a file in `BMO_TESTBED_TIMES`:** every turn taken
  under it adds one line there: the command, its lane, its wait and its run. A static check reads its
  steps' waits back that way.

## A run on another machine

- **A heavy run may be placed on another machine** before it asks for its turn, unless it was started
  inside a turn already held or inside a whole run's unit. Where it goes is the machine's placement
  (`docs/boundary.md`, 「Placement」): the placed machine its settings name, reached over `ssh`, or the
  placement endpoint's answer; with neither, or one that does not answer, the run stays on the machine
  it was started on.
- **There it runs in a bench copy of the caller's checkout under that machine's own testbed:** the
  caller pushes its commit into a clone of the checkout's remote in the bench, copies its open files
  over, ignored ones left out, and copies every tree the checkout's workspace names outside the
  repository to its own place under the bench root; the checkout's setup then runs there, and the
  command in that machine's own turn, on its own seats and its own memory gate. Its output and its
  one result line come back to the caller.
- **A deployment's gate and its Cargo release build go to the placed machine** the settings name
  (`docs/deploy.md`, 「Where a deployment runs」), so the machine that serves is not the machine that
  checks and builds.
- **A caller keeps its place in the line there:** one standing at the front (「The front of the line」) or
  next (「Next after the front」) here stands the same on the placed machine, the line run there carrying
  `BMO_TESTBED_FRONT` or `BMO_TESTBED_NEXT` as it stands here. A deployment's gate and release build
  therefore take that machine's turn ahead of every run placed there that is no deployment's.
- **A run started on a placed machine stays there.**
- **A machine that drops it** ends the run as not run, exit code 75, and the same command run again
  goes where the placement sends it next.

## Seeing the queue

- **The `status` verb** prints each lane: its width, who holds each seat held and since when, a line
  for each seat, or nobody, and who waits behind them in order, with the front and the next marked;
  then the close line of the
  repository it is run in, its holder and who waits behind it. `--json` gives the same as structure
  (`docs/boundary.md`).

## Languages

- **A language is one member of the language type in `core`:** a module of its own, answering what
  the common type asks, and named once in the list of members beside it. The run, its result and the
  `classify` verb ask that one list, so nothing else names the languages.
- **What a language answers:**
  - its name, which the result line shows and `--adapter` forces;
  - whether it claims a command, read from the program and the words after it as the shell split them;
  - the command as it runs in its turn: its share of the width handed to the runner, through an
    argument or the environment;
  - what a run's output adds up to: how many tests passed, failed and were skipped;
  - what failed, as short lines out of the output;
  - for a language that runs test files, its own naming of a test file, and the command that runs
    test files, handed the caller's words, with no per-test limit of its runner's own standing, and each
    file by its path from the folder the command runs in;
  - for a language whose runner reads a configuration file of its own from the folder it starts in,
    that file's name, which decides the folder its test files run from (`docs/suite.md`, 「Which
    files run」);
  - for a language that runs package scripts, the command that runs a project's script;
  - for a language of a package workspace, its root configuration, the files at the repository's root
    a change to which reaches every package, and the name of its lockfile, which marks a folder
    carrying it beside a package manifest as a project of its own (`docs/checks.md`, 「Which tests a
    change runs」).
- **The ones there are:**
  - `bun`: `bun test` over anything but one test file; its test files are named `*.test.ts`, run by
    `bun test` handed each file as a path from where it runs (a word that is not written as a path
    is a filter bun matches within every test file's path) and `--timeout 0`, which lifts bun's own
    per-test limit unless the words name one, its configuration is `bunfig.toml`, and
    it runs a project's script with `bun run`. Its root configuration is `package.json`, `bun.lock`,
    `bunfig.toml` and `tsconfig.json`, and its lockfile `bun.lock`;
  - `cargo`: build, check, clippy, test, bench, doc and nextest;
  - `node`: package scripts that test, check or build under npm, pnpm, yarn or bun; `node --test`;
    Vitest; Jest. Its test files are named `*.test.js`, `*.test.mjs` and `*.test.cjs`, run by
    `node --test`, and it runs a project's script with `npm run`. Its root configuration is
    `package.json`, `package-lock.json` and `tsconfig.json`, and its lockfile `package-lock.json`;
  - `browser`: Playwright, and a `bun test` file under a `browser` directory, which it runs as bun
    does, by bun's configuration;
  - `cargo` and `node` read no configuration file of their own, so their test files run from the
    repository's root; `cargo` and `browser` name no root configuration and no lockfile, since a
    Cargo workspace is read through `cargo metadata` and a browser file stands in bun's workspace.
- **A command no language claims** still runs through the testbed when it is started there, read by
  the plain language, `plain`: its exit code, its time, and the last lines of its output on a
  failure.

### Adding a language

1. Write one module in `core`'s languages, named after the language, a member of the common language
   type, and add it to the list beside them.
2. Claim only what is heavy: a run that compiles a project or runs many tests. A light command claimed
   there is refused by the hook and queued behind every suite on the machine.
3. Widen through the runner's own handle and leave a number the command already names alone.
4. Read counts from the runner's own summary lines and sum every block, since many runners print one
   per file or binary.
5. Where it runs test files or package scripts, answer its naming and its commands for them, and
   where its runner reads a configuration file of its own, that file's name; for a package
   workspace, its root configuration and its lockfile.
6. Put a test in `core`'s tests that holds a claimed command, an unclaimed one, the widening, a
   summary read back, and its naming, commands, configuration and workspace files where it has them.
7. Nothing else changes: `classify`, the run, its result, the suite and the check read the list.
   Adding a language is a minor version (`docs/boundary.md`, 「Versions」).
