# The suite: running a repository's whole test suite to the end

## What it is

- **The `suite` verb runs every test unit of a repository,** or the ones a change reaches
  (`docs/checks.md`, 「Which tests a change runs」), and always ends inside the one call that started
  it: passing, failing, or naming the units that would not end or never started.
- **What a unit is:** a test file of the repository's declared runner, a crate of its Cargo
  workspace, a project of its own, or a declared package check (`docs/declaration.md`). Each runs in a
  process of its own.
- **Why a process per file:** a per-test timeout cuts only a test that yields to the timer. A test
  stuck in a synchronous loop, a native call or an import holds its whole process; inside one shared
  process it would hold every file after it. A file in a process of its own is cut by its file guard
  whatever holds it.
- **Every caller runs the same verb:** a person, a close, a deployment's check and the daily whole
  run. A run placed on another machine runs the same verb there (`docs/testbed.md`, 「A run on another
  machine」).

## One sweep, a process per file

- **Every test file runs in its own runner process,** inside its own file guard, as many at once as
  the sweep is wide.
- **Width:** the testbed's wide lane, the machine's thread count (`docs/testbed.md`, 「The width」). A
  run holds the wide lane's turn, so no other suite or build shares the machine with it, only the
  narrow lane's one command of a few threads.
- **The largest files start first.** A file's size says roughly how many tests it holds, so the long
  runs start while the rest can still fill the places around them.
- **A file's process takes its descendants with it.** Each file runs as the head of a process group of
  its own, and when the file ends or its guard fires the whole group is taken down.
- **Streaming:** each file's result is written out whole as that file ends, its verdict ahead of it,
  so what the run found is already in the redirected file even if the run is killed from outside.
  Output is held per file because files running beside one another would otherwise interleave.

## Browser files run in a lane of one

- **The files the declaration names as browser files** run in the same sweep, one at a time, in a lane
  of their own beside the other files. The lane starts with the run, so the browser files do not wait
  for the rest and the rest do not wait for them.
- **Why one at a time:** each drives a browser, and several browsers side by side slow one another far
  more than they slow the rest.
- **Why a process each:** a browser driver freshly loaded several times inside one process can hang
  where the browser starts or closes; a process per file loads it once.

## Heavy units run in a lane of one

- **A crate's tests, a project's check and a declared package check** each run as one entry of the
  sweep, as the head of a process group of its own inside a file guard, in a second lane of one
  beside the browser lane. Each spends the machine's threads on its own, so two never run side by
  side; the projects go down the lane first, then the crates.
- **Their verdicts stand in the log as a file's do,** and the totals count Cargo's result lines with
  the runner's, ignored tests as skipped.
- **Crates are built once, before the guards start.** Every crate to be tested is built once the run
  has its turn and before its seconds are counted, by one Cargo command per crate with the words its
  run will use, so the run compiles nothing inside a guard. A crate with a library builds its
  library's own tests first, alone, then the whole crate, so the two heaviest compiles never run side
  by side. A build that fails fails each unit again, by name.
- **Cargo is handed as many jobs as the fence's memory holds** at a gigabyte and a half each, never
  more than the run's width nor fewer than one; a caller that named fewer keeps its own. The run says
  the number under its first line.
- **Each build ahead that passed removes what it superseded** (`docs/deploy.md`, 「The Cargo build
  directory」).
- **A test that drives a crate's binary** asks the test kit for it by name; the binary is built once
  per run and its path in the checkout's own target directory handed back, so the test runs the code
  of the checkout it stands in.

## One run at a time on a machine

- **A run takes the testbed's wide lane before it starts,** with the testbed's waits
  (`docs/testbed.md`, 「Waiting never fails a command」); `--turn-wait` sets another. A caller no job's
  call bounds passes twenty-five minutes.
- **Inside a turn its caller holds, a run starts at once.**
- **Where the turn has not come, the run does not start.** It writes its turn line, naming its place
  and the holder, and a verdict saying it was not run, and ends with exit code 75. Its place is kept,
  so the caller runs it again at once and waits on from where it stood.
- **The wait is not the run's own time.** The guards and the verdict's seconds start once the turn has
  come.

## A fence of its own

- **Once its turn has come, a run moves itself into a transient scope of its own,**
  `bmo-suite-<pid>.scope`, before it starts a file, so every file and everything a file starts is born
  inside it. Its line under the first names the scope and its ceilings.
- **The ceilings:** eight gigabytes of memory, one of swap beyond it, and 8,192 tasks.
  `BMO_SUITE_MEMORY_GB` sets the memory ceiling for a machine where eight is the wrong number.
- **Why:** without a scope a run's processes count against whatever unit its caller stands in, and a
  runaway test fills the whole machine, where the kernel's out-of-memory killer takes the largest
  process, seldom the runaway's. Inside the scope the runaway meets the scope's own ceiling. The
  scope keeps the run itself standing when one of its processes is killed, so the file fails and the
  verdict is still written.
- **Every process of the run offers itself first** to the out-of-memory killer: the run raises its
  own score to the most, and every process it starts inherits it.
- **Where no user service manager can be reached,** inside a test's home above all, the run says so on
  the same line and runs where it stands.
- **The run leaves its scope empty.** As its last act before the verdict, a run standing in its own
  scope kills every other process still in it and names them on one line. A run standing where its
  caller stands kills nothing.

## The guards

- **Thirty seconds per test** that names no limit of its own, handed to the runner; a caller that
  names one keeps its own.
- **Ninety seconds per file.** A file that has not ended by then is cut: its process group is killed,
  its result line reads `CUT` with the seconds it ran, and what it wrote before the cut follows.
- **Four minutes for the whole run.** Once they have passed, no further file is started; the files
  already running go on under their own guards. The run then names the files it never started.
- **A cut file fails the run,** and the verdict names it as one that would not end.
- **A file that never started is not a failure.** Nothing of it was tested, so it is counted apart:
  - where nothing failed and nothing was cut, the run ends with a failed verdict saying how many never
    started and that none failed, and with exit code 75, the code of a run to make again;
  - where something also failed, it ends with exit code 1, the files that never started still named
    apart.
- **Why so far above the work:** the machine a run stands on carries other work as well, and a guard
  set close to the measured time turns a loaded run into a cut one, whose only answer is to run the
  same tests again.
- **Changing them:** `--guard` sets the whole-run guard and `--file-guard` the per-file one, in
  seconds.

## Which files run

- **Git answers:** tracked test files and test files not yet committed, matching the declared test
  file globs, outside every project of its own. Uncommitted files matter most: a newly written test
  is the usual cause of a stalled suite.
- **A word naming a path narrows the run** to the test files whose path holds it, and runs no heavy
  unit, except a word that is a crate's or a project's unit name exactly, its directory and a slash,
  which picks that unit and no test file.
- **A selected run keeps only the units its change reaches** (`docs/checks.md`), and a word naming a
  path narrows that further.

## The test database

- **A repository that declares a test database** (`docs/declaration.md`) has its database tests run
  on a PostgreSQL server of the suite's own, apart from any server the repository's services run on,
  so a run takes no connection a service needs.
  - It is a cluster of the user's own under `~/.local/state/bmo-test-postgres`, reached through a
    socket only the user can open. The first run that asks for it makes it; a run that finds it not
    answering starts it as the transient user unit `bmo-test-postgres`, which outlives the run.
  - What it holds is thrown away with every test, so it waits for no disk, and it takes three hundred
    connections. It is made with the `C` collation, and so is every database in it.
  - The suite names it in `BMO_TEST_POSTGRES` for every process it starts. Naming another address
    there before the run puts the run on that server instead, which the suite then neither makes nor
    starts.
- **The template is named by the migrations:** `bmo_test_tmpl_<hash>`, the hash of the declared
  migration files. Every run and every checkout whose migrations are the same shares one template and
  its pool. A repository declaring no migrations has no template and no pool; its tests are handed the
  server alone.
  - It is made by the repository's declared migrate command, handed the template's address. A run
    makes it before its first file starts where it does not stand yet; otherwise the first file to ask
    makes it, under a lock the others wait on. The `template` verb is behind both
    (`docs/boundary.md`).
- **The pool is the template's clones,** `bmo_test_<hash>_s<n>`, kept from run to run.
  - A file's process claims the first slot no living process holds, by a claim file in the temporary
    directory naming its process, written whole and linked into place. A claim whose process is gone
    is taken over.
  - A slot that stands is emptied as it is claimed; one that does not is cloned from the template.
  - Nothing is dropped as a file ends: dropping or making a database waits on a checkpoint.
- **The sweep:** every run starts the `template sweep` with its own template's name. It drops every
  other template made more than a day ago whose pool no living process holds, with that pool.
- **A Rust test's own database** comes from the `scratch-database` crate, handed the migrator of the
  crate under test:
  - The first test of a binary to ask makes the binary's template, `bmo_rs_tmpl_<process>_<n>`, one
    for each migrator handed in, and migrates it; each test is handed a copy,
    `bmo_rs_copy_<process>_<n>`, which the server makes in a fraction of the time a migration takes.
  - A copy is dropped as its test ends, its connections ended with it. The first template a binary
    makes drops every template and copy whose process is gone.
  - The server is the one `BMO_TEST_POSTGRES` names, which the suite sets; with nothing named, the
    machine's own PostgreSQL socket.
- **At most ten files that use the database run at once,** whatever the sweep's width.

## The test home

- **Every test file runs in a home of its own:** a temporary directory, `bmo-test-home-<random>`, that
  is `HOME`, set up before the file's first line, so no test reads or writes the account's real home.
  - A consumer's own preload sets it up, handing the test kit what it asks for: the variables to take
    away, the programs to stand in for, and the directories of the home to name in variables. The
    lists are the consumer's; the test kit names none.
  - The home is named in `BMO_TEST_HOME`, so a process that carries it is a test. `XDG_CONFIG_HOME`
    is a directory of the home.
- **The variables a consumer names are taken away:** set empty where a reader takes empty as unset and
  an env file would otherwise hand the value back; removed, by name or by the beginning of the name,
  where the reader takes a present empty value as given.
- **What still comes from the real home,** named where it stands so that moving the home does not move
  it, unless the caller named it already: git's global configuration, so a test committing in a
  temporary repository keeps its author; the user cache; the browser builds the browser lane uses.
- **No user service manager:** `XDG_RUNTIME_DIR` is a directory of the home holding no socket and
  `DBUS_SESSION_BUS_ADDRESS` is removed, so a test reaches no service manager unless it stands one up.
- **The machine's turn is the home's:** `BMO_TESTBED_LOCK` names a lock in the home and
  `BMO_TESTBED_HELD` is removed, so a heavy command a test starts takes its turn there and never waits
  on the run the file stands in.
- **Stand-ins:** the home's `bin` goes at the front of `PATH`, with a stand-in for each program the
  consumer names. A stand-in writes that it was called, with its arguments, into the home's `refused`
  file, and exits 97; at the end of the file the record is read, and a file whose test called a
  stand-in fails, naming it, though its own tests passed.
- **Launches use the current environment:** starting a program and looking one up default to the
  environment as it stands, so a program a test starts finds the home and the stand-ins.
- **A home left by a file that died** is cleaned up by the next file after an hour.
- **Nothing reaches for the process's standard error as it loads:** a library building a stream on it
  while a worker holds an earlier one is refused, and the file whose import caused it drops out
  uncounted. The test kit's stand-ins preload answers the `debug` library's colours question, and
  stands in, at the consumer's asking, for a module that would take standard error outright, which
  then refuses whatever is asked of it.
- **A scratch directory** a test asks the test kit for, under a name beginning `bmo-`, is taken away
  as its file ends, whatever became of its tests. A directory a killed run left under the same
  beginning is taken away an hour on; only directories the test kit made, never a file.

## What failed runs once more

- **A test failing only now and then lets a change through once the next run passes,** and is mended
  rather than forgotten. A close and a deployment never stall on one.
- **With `--again`,** where the first run failed or left files unstarted, the files and units that
  failed or never started run once more, alone and by name, in the same tree. The run says so first,
  naming them.
  - It runs once more at most, and judges on that second run: where it passed, the run passes; where
    it did not, it fails as the second run ended, with what failed in either run named.
  - A run whose turn never came and left nothing unstarted has nothing to run once more.
- **What passed the second time is flaky:** each test the first run named as failing, with the file or
  unit it stands in, and each file or unit that failed. One that only never started is not flaky.
- **The flaky record:** one line in `suite-flaky.jsonl` under the data root's `testbed/` for each run
  that found any, with the time, the commit, the tests by name with their files, the files and the
  log, and a `suite-flaky` event goes to the hook (`docs/boundary.md`, 「Events」).

## The daily whole run

- **Why it exists:** a selected run trusts what each package declares. A process launched through a
  path assembled from pieces, or an import no static reading finds, is a dependency no declaration
  names. Once a day every test runs regardless, so such a gap costs at most a day.
- **What it is:** the `full` verb puts a detached worktree of its own on the main branch's head, sets
  it up with the repository's declared setup, runs the static check, and runs every unit through the
  suite, waiting its turn like any other run.
  - The tree is `full-check` beside the repository's deploy tree, or wherever `BMO_FULL_CHECK_DIR`
    names.
  - Its log is kept under `suite-full/` in the data root's `testbed/`, one file per run.
  - One run at a time per repository: a run holds a lock under the data root's locks, and a second
    meeting it is refused.
- **Where it runs:** never in its caller's process. Called from anywhere but its own unit, it starts a
  transient unit, `bmo-full-suite-<stamp>`, and ends.
- **When:** whenever the consumer's scheduler starts it. A run writes its start time into
  `suite-full/started` before anything else, and `full --due <HH:MM>` starts one only where none
  started since the latest such time of day, so a scheduler that asks every minute runs it once.
- **Files that never started run again.** Where a pass leaves files unstarted, it runs again over those
  files alone, and again while each round starts more of them than the one before. The pass stands on
  what every round came to.
- **What it tells:** a run that fails sends a `full-suite-failed` event naming the failing files, the
  unit each stands in, the head it ran at and the log; a tree that could not be set up, or a static
  check that failed, is told the same way with what failed. A run that passes says nothing beyond its
  log.

## The lines the run writes about itself

- **Both open with `suite:`,** so a caller holding a log full of the runners' own output can find them.
- **The first line says what the run holds itself to,** the two guards, so a log cut off still says
  at its head what the run was allowing.
- **The last line is always the verdict:** whether it passed, how long it took, and what happened. Its
  presence is the answer to whether the run has ended.

## Reading the output

- **Redirect it to a file and read the file.** Filtering through a pipe loses everything the pattern
  missed, and the only way back is to run the whole thing again.
- **The run is waited on, never watched.** Start it in one call whose own wait covers it, and read the
  file after the call returns.
- **The `log` verb** answers the three questions of a log: how many passed, how many failed, and what
  failed. The totals are the sum of every summary block in the log, not the last one. It reads a
  single file's runner log as well. It ends non-zero on a failure.
