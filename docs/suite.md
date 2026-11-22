# The suite: running a repository's whole test suite to the end

## What it is

- **The `suite` verb runs every test unit of a repository,** or the ones a change reaches
  (`docs/checks.md`, 「Which tests a change runs」), each to its end, and ends on a verdict: passing or
  failing, naming the units that failed.
- **What a unit is:** a test file of the repository's declared runner, a crate of its Cargo
  workspace, a project of its own, or a declared package check (`docs/declaration.md`). Each runs in a
  process of its own.
- **Why a process per file:** a test stuck in a synchronous loop, a native call or an import holds its
  whole process; inside one shared process it would hold every file after it. A file in a process of its
  own holds only itself, and the files beside it run on and write their results.
- **Every caller runs the same verb:** a person, a close, a deployment's check and the daily whole
  run. A run placed on another machine runs the same verb there (`docs/testbed.md`, 「A run on another
  machine」).

## One sweep, a process per file

- **Every test file runs in its own runner process,** until it ends, as many at once as the sweep is
  wide.
- **Width:** the run's share of the testbed's wide lane (`docs/testbed.md`, 「The width」): the
  machine's thread count divided by the lane's seats for the one seat a run holds, a close's in one of
  the lane's seats and a deployment's in its own. The commands in the lane's other seats share the
  machine with it, as does the narrow lane's one command of a few threads.
- **The largest files start first.** A file's size says roughly how many tests it holds, so the long
  runs start while the rest can still fill the places around them.
- **A file's process takes its descendants with it.** Each file runs as the head of a process group of
  its own, and when the file ends the whole group is taken down.
- **Nothing of the testbed ends a process while the unit that started it runs.** A file's group goes once
  the file's process has ended, a heavy unit's once the unit has ended (「Heavy units run in a lane of
  one」), and whatever the run's fence still holds once the whole run has ended (「A fence of its own」).
  A test runner ends processes of its own inside a unit: Bun's, where a test or a hook runs past its
  bound, ends every process its file started, a browser and a server the file's tests share among them,
  and says `killed N dangling processes`. Every later test of that file then finds them gone, so a
  file's tests that share a browser or a server bound each wait well under the test's own bound, and the
  line is read as the end of a test that ran out of time, never as the testbed's.
- **Streaming:** each file's result is written out whole as that file ends, its verdict ahead of it,
  so what the run found is already in the redirected file even if the run is killed from outside.
  Output is held per file because files running beside one another would otherwise interleave.
- **A unit says how its process ended where its output cannot:** one ended by a signal names the signal
  under its heading, and one that failed having said nothing names the exit code it ended with, so a
  process gone before its runner printed a word still leaves an account in the log.

## Browser files run in a lane of one

- **The files the declaration names as browser files,** and the ones each project of its own the run
  checks declares as its own (`docs/checks.md`, 「Which tests a change runs」), run in the same sweep, one
  at a time, in a lane of their own beside the other files. A project's browser file runs as any test
  file does, by the declared language that runs it, from the folder of its runner's configuration. The lane starts with the run, so the browser files do not wait
  for the rest and the rest do not wait for them.
- **A project's browser file starts once its project's check has ended,** whatever the check's verdict.
  The check is what readies the project: it installs the project's locked packages and makes whatever
  else its files import, and a tree freshly set up, such as a bench copy on another machine, holds none
  of it until then. While a project's browser file waits, the lane takes the next browser file that
  waits on nothing, so the declaration's own browser files run meanwhile; a lane whose every file
  waits holds its place until a check ends.
- **Why one at a time:** each drives a browser, and several browsers side by side slow one another far
  more than they slow the rest.
- **Why a process each:** a browser driver freshly loaded several times inside one process can hang
  where the browser starts or closes; a process per file loads it once.

## Heavy units run in a lane of one

- **A crate's tests, a project's check and a declared package check** each run as one entry of the
  sweep, as the head of a process group of its own, in a second lane of one
  beside the browser lane. A project standing inside another is an entry of its own, its check run in
  its own folder beside the outer one's. Each spends the machine's threads on its own, so two never
  run side by side; the projects go down the lane first, then the crates.
- **Their verdicts stand in the log as a file's do,** and the totals count Cargo's result lines with
  the runner's, ignored tests as skipped.
- **The crates' tests are built once, by one Cargo command naming every crate the run reaches,** once
  the run has its turn and before its seconds are counted, so no unit compiles inside the sweep. Cargo
  settles the features of each dependency over all the packages one command names, so one command
  builds a crate the others share once, where a command per crate would build it again for every set
  of features.
  - **The crates the run reaches** are every crate of the workspace on a whole run and the ones the
    change marked on a selected run, before any word narrows the run to some of them. A run naming one
    crate's unit stands on the same build as the run it narrows and builds nothing again where nothing
    changed. A run whose words leave it no crate builds none.
  - **The libraries' own tests go first, alone,** by the same command over the same crates kept to their
    libraries, then everything else, so a library and its own tests, a crate's two heaviest compiles,
    never run side by side.
  - **The build names its tests by Cargo's `--tests`:** each library's and each binary's own tests and
    every integration test. A crate's documentation examples are not run as tests.
- **Cargo's own runner writes each test binary down.** The second command runs the tests as Cargo runs
  them, with a runner of the testbed's own (`crate-tests record`, `docs/boundary.md`) that runs no test:
  for each test binary it writes down the binary, its package, the folder Cargo runs it from and the
  variables Cargo hands it, `CARGO`, `CARGO_MANIFEST_DIR`, `CARGO_MANIFEST_PATH`, the package's
  `CARGO_PKG_` values and the library path, into a record of the run's own, and ends at once. The
  record goes with the run.
- **A crate's unit runs the test binaries written down for it** (`crate-tests run`), one after another,
  each to its end, from the folder Cargo would run it from and with the variables Cargo would hand it
  laid over the unit's own environment. It fails where any of them failed, on the first failure's code,
  and a crate with no test binary passes, saying so. A build that failed fails each crate's unit by name,
  the build's own output standing above them in the log.
- **The lock's path packages follow their manifests for the whole run.** A run that tests a crate
  brings the workspace lock's path packages to their manifests before its first build, as the Cargo
  verb does around a command holding its lock, says on one line which moved, or why none was moved, and
  keeps the lock so until its last unit has ended, when it is put back as it was found (`docs/deploy.md`,
  「The Cargo build directory」). The crates' builds go through the Cargo verb, which aligns the lock around its own
  command alone, while the crates' tests run outside it and may run Cargo themselves, so a sibling's new
  version fails neither.
- **Cargo is handed as many jobs as the fence's memory holds** at a gigabyte and a half each, never
  more than the run's width nor fewer than one; a caller that named fewer keeps its own. The run says
  the number under its first line.
- **Each build ahead that passed removes what it superseded** (`docs/deploy.md`, 「The Cargo build
  directory」).
- **A crate's binaries are built for the test files only where a package asks for them.** A package
  whose tests start a crate's binary names the crate's directory in its `runs` (`docs/checks.md`, 「Which
  tests a change runs」). A run that starts a test file builds, once, the binaries of every crate so
  named: by every package on a whole run, by the packages the change marked on a selected run, and by
  none on a run a word narrows. A test finds the binary in the checkout's own target directory, so it
  runs the code of the checkout it stands in. A crate's own integration tests have its binaries built
  by Cargo with them.

## One run at a time on a machine

- **A run takes the testbed's wide lane before it starts,** waiting for it however long the line takes
  (`docs/testbed.md`, 「Waiting never fails a command」).
- **Inside a turn its caller holds, a run starts at once.**
- **Where the harness it runs for goes away while it waits, the run does not start.** It writes its turn
  line, naming its place and the holder, and a verdict saying it was not run, and ends with exit code 75.
- **The wait is not the run's own time.** The verdict's seconds start once the turn has come.

## A fence of its own

- **Once its turn has come, a run moves itself into a transient scope of its own,**
  `bmo-suite-<pid>.scope` in `bmo-suite.slice` (`docs/deploy.md`, 「The slices」), before it starts a
  file, so every file and everything a file starts is born inside it. Its line under the first names the scope and its ceilings.
- **The ceilings:** eight gigabytes of memory, one of swap beyond it, and 8,192 tasks.
  `BMO_SUITE_MEMORY_GB` sets the memory ceiling for a machine where eight is the wrong number. Its open
  files are the machine's ceiling, as for every verb (`docs/testbed.md`, 「Open files」).
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

## How long a unit and a test may run

- **A unit is never cut for its length as a whole.** A file, a crate's tests, a project's check and a
  declared package check each run until they end, however long that takes, and every unit is started.
- **A single test is held to the per-test limit:** the declaration's `test_timeout_seconds`, else the
  declaration policy's, 120 seconds. A test that waits on something that never comes, a stand-in nobody
  answers or a lock nobody lets go, fails at that limit rather than holding its unit until the silence limit
  below, which is five times as long and counts only from the unit's last word.
- **A browser file's runner is handed no per-test limit, the testbed's or the runner's own.** The files the
  declaration names as browser files and the ones each project of its own declares (「Browser files run in a
  lane of one」) share one browser across the whole file, and a runner that ends a test past its bound ends
  every process that file started with it, the browser among them (「One sweep, a process per file」), so
  every later test of that file loses the browser it drives. What bounds such a file is the waits its own
  tests name, and a test that waits on something that never comes is ended as a hung unit (below), which
  costs that file alone. Where a caller names a limit in its words, the file keeps the caller's.
- **Each language hands its runner the limit its own way** (`docs/testbed.md`, 「Languages」):
  - `bun test` is handed the limit as its own `--timeout`, so the test fails there and the file runs on. A
    caller that names a limit in its words keeps its own, and a limit a test sets for itself is the test's.
  - A crate's tests have no limit in Rust's test harness. The harness says of each test still running after
    a minute that it has been running that long, naming it; the run reads the unit's output as it comes, and
    once a test so named has run for the limit without its result, it ends the unit's process group as a
    hung unit is ended. The unit fails with exit code 124 and its output closes on a line naming the test
    and the limit, so a deployment's gate says which test hung. A limit below the harness's minute takes
    effect at that minute.
  - A project's check and a declared package check run their own runners, whose own limits stand.
- **What fails at the limit runs once more** as any failing unit does (「What failed runs once more」).
- **A unit that neither writes nor works for the silence limit is ended as hung** (`docs/testbed.md`, 「A holder
  whose command falls silent lets go」): the run watches the pipes it reads the unit's output through, ends
  the unit's process group once they stay silent and the unit's processes burn no processor time, and the unit fails with exit code 124, the line naming it
  in its output. While a unit writes or works, the run touches the file its own holder handed it in
  `BMO_TESTBED_HEARD`, since the unit's output reaches the run's own only once the unit ends. A unit that
  writes but never ends holds the run open until someone stops it.
- **A unit's output closes once it has ended:** whatever it left standing in its process group is killed
  at once, and so is any process outside the group still holding its output, so the run reads the output
  whole and goes on. A process the run started itself is never taken for such a holder: one the run is
  still starting holds every output the run has open until it becomes its own program, though it is no
  leftover of the unit that ended. The run is no subreaper, so nothing a unit leaves behind is the run's
  own child.
- **Keys read and given no effect:** `--guard`, `--file-guard` and `--turn-wait`, and a declaration's
  `guard_seconds` and `file_guard_seconds`.

## A skipped test is said

- **The verdict names how many tests the units skipped,** read from each unit's own output by the counts
  the `log` verb reads (「Reading the output」), whatever kind of unit printed them: a test file, a
  project's check script, a tool's check or a crate's tests. A run that passed with tests skipped says so
  on its last line, so a skip never reads as a bare pass.
- **A test that needs a program the machine lacks is the test's to skip,** saying which program and why,
  in its own output; the suite counts the skip like any other. The testbed installs nothing on a machine,
  the one it was started on or the one it placed the run on: a program a repository's tests need is
  installed on every machine its runs go to, and until it is, those tests stand skipped in every verdict.

## Which files run

- **Git answers:** tracked test files and test files not yet committed, matching the declared test
  file globs, or where none are declared each declared language's own naming, outside every project
  of its own. Uncommitted files matter most: a newly written test is the usual cause of a stalled
  suite.
- **The declared languages run them** (`docs/testbed.md`, 「Languages」): each file by the first
  declared language whose naming it carries, or else the first that runs test files, handed the
  caller's words, with no per-test limit of the runner's own standing. A repository declaring no language that runs
  test files has none; a project's check script is run by the first declared language that runs
  package scripts.
- **A project of its own runs its own tests,** through its check script and in no other way: the files
  inside it are none of the run's files, apart from the browser files it declares. A project whose check
  script runs no tests has its tests run nowhere, so a project holding tests runs its runner in that
  script, after its static check (`docs/checks.md`, 「Which tests a change runs」).
- **Each file runs from the folder of its runner's configuration:** the nearest folder at or above
  the file, inside the repository, holding the configuration file its language reads (bun's
  `bunfig.toml`), or the repository's root where none does or its language reads none. It is handed
  by its path from that folder. A runner reads its configuration only from the folder it starts in,
  so a file run from anywhere else would lose what that configuration sets up, such as a preload
  giving each file a test home of its own.
- **A word naming a path narrows the run** to the test files whose path holds it, and runs no heavy
  unit, except a word that is a crate's or a project's unit name exactly, its directory and a slash,
  which picks that unit and no test file.
- **A selected run keeps only the units its change reaches** (`docs/checks.md`), and a word naming a
  path narrows that further.

## What a test unit is handed

- **The deployment's own variables are taken away from every test.** A suite run by a deployment's gate
  carries the deployment's environment, naming the live data root, the backups, the deployment record and the
  markers of the run underneath it. Those names stand under `[environment] stripped` in `policy/suite.toml`
  and are removed from every unit's environment — a project's check script, a tool's check and a crate's
  tests alike — so a test reads its own environment and never the deployment's.
- **What the suite names for a unit itself is handed after that list is taken away:** the turn it holds, the
  test server and its template, and the Cargo jobs and target the build reads.

## The test database

- **A repository that declares a test database** (`docs/declaration.md`) has its database tests run
  on a PostgreSQL server of the suite's own, apart from any server the repository's services run on,
  so a run takes no connection a service needs.
  - It is a cluster of the user's own under `~/.local/state/bmo-test-postgres`, reached through a
    socket only the user can open. The first run that asks for it makes it; a run that finds it not
    answering starts it as the transient user unit `bmo-test-postgres` in `bmo-suite.slice`, which
    outlives the run. A run that finds another making or starting it waits for that one, however long
    it takes, saying once a minute that it waits.
  - What it holds is thrown away with every test, so it waits for no disk, and it takes three hundred
    connections. It is made with the `C` collation, and so is every database in it.
  - On PostgreSQL 18 or later it does its reads and writes in the backend that asks for them (`io_method`
    set to `sync`) rather than in separate I/O workers. Those workers sit idle while the server is
    otherwise quiet and can be slow to accept the signal every dropped database sends to every
    process, and a drop waits for them all, sometimes for minutes. A server older than 18 has neither the
    setting nor the workers, so it is started without it. The settings and the version each one needs are
    listed under `[server]` in `policy/suite.toml`.
  - A run, or a change check about to run crate tests, that finds the server answering with settings
    other than these restarts it, but only where it runs as the `bmo-test-postgres` unit, no other session
    is connected to it and no other run's fence stands. Otherwise the run uses it as it stands and says
    so, and the first later run that finds it idle restarts it. A file asking for its template never
    restarts it.
  - The suite names it in `BMO_TEST_POSTGRES` for every process it starts. Naming another address
    there before the run puts the run on that server instead, which the suite then neither makes nor
    starts.
- **The template is named by the migrations:** `bmo_test_tmpl_<hash>`, the hash of the declared
  migration files. Every run and every checkout whose migrations are the same shares one template and
  its pool. A repository declaring no migrations has no template and no pool; its tests are handed the
  server alone.
  - The suite names the template in `BMO_TEST_TEMPLATE` for every process it starts, so a file's
    process claims its slot from that template's pool.
  - It is made by the repository's declared migrate command, handed the template's address. A run
    makes it before its first file starts where it does not stand yet; otherwise the first file to ask
    makes it, under a lock the others wait on. The `template` verb is behind both
    (`docs/boundary.md`).
  - It is made under another name it takes only once whole, so a making cut short is never taken for
    it, and noted with the time it was made, which the sweep reads.
  - It carries two functions a clone is reused with: `bmo_test_reset()` empties every table a test
    wrote and drops every schema named `t_…`, and `bmo_test_count_on()` sets every identity counting
    on past the largest key its table holds.
- **The pool is the template's clones,** `bmo_test_<hash>_s<n>`, kept from run to run.
  - A file's process claims the first slot no living process holds, by a claim file in the temporary
    directory naming its process, written whole and linked into place. A claim whose process is gone
    is taken over.
  - A slot that stands is emptied as it is claimed; one that does not is cloned from the template.
  - Nothing is dropped as a file ends: dropping or making a database waits on a checkpoint.
- **The sweep:** every run starts the `template sweep` with its own template's name. It drops every
  other template made more than a day ago whose pool no living process holds, with that pool, and
  every scratch database whose process, the one its name carries, is gone.
- **A Rust test's own database** comes from the `scratch-database` crate, handed the migrator of the
  crate under test:
  - The first test of a binary to ask makes the binary's template, `bmo_rs_tmpl_<process>_<n>`, one
    for each migrator handed in, and migrates it; each test is handed a copy,
    `bmo_rs_copy_<process>_<n>`, which the server makes in a fraction of the time a migration takes.
  - A test that needs a database with nothing migrated is handed a blank one, made from `template0`
    under the copies' name and dropped and swept as a copy is.
  - A copy is dropped as its test ends. The drop first ends every session of the test role on it, then
    drops it without force, which asks the server's own maintenance on it to stop: a forced drop would
    have to end that maintenance, which a role that is no superuser may not. It is asked again a few
    times, and a copy still standing fails its test with the server's reason.
  - The first template or blank database a binary makes first drops every template and copy whose
    process, the one its name
    carries, is gone from this machine, that the test role may drop and that no client is connected
    to, the same way; one still standing fails that test. It touches no other name, so a repository's
    own `bmo_<repository>_test_` databases and the suite's pool are never its.
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

- **A test failing only now and then lets a change through once its unit passes when run once more,**
  and is mended rather than forgotten. A close and a deployment never stall on one.
- **With `--again`, a unit that ran and failed runs once more at once,** as soon as it ends, in the
  place of the sweep it held and before that place takes another unit: a heavy unit's second run stays
  in the lane of one, a browser file's in the browser lane, and a project's browser files wait for it
  as they wait for the project's check. The run's verdict waits for no other unit, so the whole run
  takes no longer than its slowest unit with its second run.
  - A unit runs once more at most, and stands on that second run: where it passed, the unit passes;
    where it did not, the unit fails.
  - A unit that never ran, a project with no check, a crate whose tests did not build, is not run once
    more: nothing of it ran to fail only now and then.
- **What runs once more is what failed, as narrowly as the unit can name it:**
  - **A crate:** each test binary that failed, from the same build, so nothing is built again. A
    binary whose own result names the tests that failed runs only those, each by its full name matched
    exactly, through Rust's test harness's `--exact`; one that ended with no result of its own, ended
    by the per-test limit or by a signal, runs whole, as does one that never started because its unit
    was ended before it. A binary that passed does not run again.
  - **A project of its own:** the test files its tests' output names as holding a failed test, run as
    a test file of the repository runs (「Which files run」): by the declared language that runs them,
    from the folder of that runner's configuration inside the project, with the run's words and
    per-test limit. The steps of the check before its tests do not run again, since a project's check
    runs its tests last and ends at the first step that fails (`docs/checks.md`, 「Which tests a change
    runs」), so a check whose tests named a failed file passed every step before them. Where the output
    names no failed file, names one the project does not hold, or names files one command cannot run
    together, the whole check runs once more.
  - **A test file, a browser file or a declared package check:** the unit as it ran.
- **The log holds both runs of a unit.** The first run's account stands under the `redo` mark in
  place of `fail`, followed at once by a line opening with `suite: once more` that names the unit and
  what runs once more; the second run's account follows under its own heading once it ends. The
  readers of a log pass over an account under the `redo` mark (「Reading the output」), so the totals
  and what failed are what each unit stands on.
- **What passed the second time is flaky:** each test the first run of a unit named as failing, with
  the unit it stands in, and each unit whose second run passed, whatever the rest of the run came to.
- **The flaky record:** one line in `suite-flaky.jsonl` under the data root's `testbed/` for each run
  that found any, with the time, the commit, the tests by name with their units, the units and the
  log, and a `suite-flaky` event goes to the hook (`docs/boundary.md`, 「Events」).

## The daily whole run

- **Why it exists:** a selected run trusts what each package declares. A process launched through a
  path assembled from pieces, or an import no static reading finds, is a dependency no declaration
  names. Once a day every test runs regardless, so such a gap costs at most a day.
- **What it is:** the `full` verb puts a detached worktree of its own on the main branch's head, sets
  it up with the repository's declared setup, runs the static check, and runs every unit through the
  suite, waiting its turn like any other run.
  - The tree is `full-check` beside the repository's deploy tree, or wherever `BMO_FULL_CHECK_DIR`
    names; a repository that declares no deploy root keeps it as `tree` in its own folder below.
  - Its log is kept in the repository's own folder, `suite-full/<repository>/` in the data root's
    `testbed/`, one file per run, the repository named as its declaration names it.
  - One run at a time per repository: a run holds the repository's lock under the data root's locks,
    and a second meeting it is refused.
  - The main branch is the one the declaration's `[check] main` names, its head read as
    `docs/declaration.md`, 「The main branch's head」, says, the remote asked first.
- **Where it runs:** never in its caller's process. Called from anywhere but its own unit, it starts a
  transient unit, `bmo-full-suite-<stamp>` in `bmo-suite.slice`, and ends.
- **When:** whenever the consumer's scheduler starts it. A run writes its start time into `started`
  in its repository's folder before anything else, and `full --due <HH:MM>` starts one only where none
  started since the latest such time of day, local time, so a scheduler that asks every minute runs it
  once.
- **What it tells:** a run that fails sends a `full-suite-failed` event naming the failing files, the
  unit each stands in, the head it ran at and the log; a tree that could not be set up, or a static
  check that failed, is told the same way with what failed. A run that passes says nothing beyond its
  log.

## The dependents of a major change

- **A repository names the repositories it builds on** in its declaration's `upstreams`, by the names those
  repositories declare. An upstream names none of its consumers; the relation stands only in the consumer's
  own declaration.
- **A machine's checkouts stand in one registry,** a TOML file whose `checkouts` list, or whose
  `[[checkout]]` tables, names each checkout's path, as a string or as a table with a `path`. A path is
  absolute, begins with `~/`, or is read against the registry file's folder. The file is
  `checkouts.toml` under the data root's `testbed/` unless `BMO_TESTBED_CHECKOUTS` names another. A registry
  that does not stand holds no checkouts; one that does not read is refused. The hosted deploy tool finds the
  checkout it deploys from in the same registry (`docs/deploy.md`, 「The hosted deploy tool」), the
  `shared-versions` quick check the repository holding the shared versions (`docs/checks.md`), and the prune
  the deploy roots whose build trees it takes (`docs/deploy.md`, 「The Cargo build directory」).
- **The `dependents` verb** reads the registry and answers every checkout whose declaration names the current
  repository in `upstreams`, each by its declared name and root. It answers the direct consumers only; it
  does not follow a consumer's own consumers.
- **With `--run`** it then runs each consumer's suite through the suite runner above, from that consumer's
  root, one repository at a time in the wide lane, each with the consumer's own declaration, log and test run.
  A consumer whose suite fails does not stop the ones after it.
- **Exit codes:** 0 where every consumer passed, or there is none; 1 where any consumer's suite failed; 75
  where none failed but a consumer's suite did not run; 2 where the registry or a consumer's
  declaration does not read. The consumers are refused before any suite starts.
- **How a change is known to be major:** a caller names it by running `dependents --run`. A deployment
  decides for itself: it compares the major of the commit's `VERSION` with the version in the deployment
  record of the release live before it, and a different major runs the same `dependents --run` after its gate
  and before its backup (`docs/deploy.md`). A first deployment, a commit carrying no `VERSION`, and a record
  carrying none are not known to be major by that comparison. The testbed's own versions stay at major 0
  before its first release, so a change there is named by its caller.

## The lines the run writes about itself

- **Both open with `suite:`,** so a caller holding a log full of the runners' own output can find them.
- **The last line is always the verdict:** whether it passed, how long it took, and what happened: the
  files passed, the files failed where any did, and the tests skipped where any were (「A skipped test is
  said」). Its presence is the answer to whether the run has ended.
- **Every line goes to the run's own log as well,** under the data root's `testbed/`, which its test
  run in the record names once its turn has come (`docs/boundary.md`, 「The record」).

## Reading the output

- **Redirect it to a file and read the file.** Filtering through a pipe loses everything the pattern
  missed, and the only way back is to run the whole thing again.
- **The run is waited on, never watched.** Start it in one call whose own wait covers it, and read the
  file after the call returns.
- **The `log` verb** answers the three questions of a log: how many passed, how many failed, and what
  failed. The totals are the sum of every summary block in the log, not the last one. It reads a
  single file's runner log as well. It ends non-zero on a failure.
- **An account under the `redo` mark is passed over,** from its heading to the next unit's heading or
  the next line opening with `suite:`: a unit run once more stands on its second run (「What failed runs
  once more」), so the totals, the failed tests and the failed units are read from that run alone.
