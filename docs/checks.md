# Checks: which tests a change runs, the static check and the quick checks

## Which tests a change runs

- **Declared by each package itself, never by a central table.** Adding a package touches only that
  package. The testbed reads the repository's own manifests: the root workspace list and each
  package's manifest for a Bun or Node workspace, `cargo metadata` for a Cargo workspace.
- **What a package declares,** in a `bmo` section of its own manifest (a crate in
  `[package.metadata.bmo]`):
  - `reads`: globs of paths outside any package that its tests read, such as a document or a prompt.
    A glob another package claims does not mark this one, so every such read is declared;
  - `runs`: files of another package it launches as a process, or a crate's directory whose binary its
    tests start, which the suite builds for its test files only where a package names it so (`docs/suite.md`,
    「Heavy units run in a lane of one」);
  - `always`: the package is selected on every change;
  - `check`: a command checking a part of the package that is not covered by its runner, run inside the
    package's folder through `sh`, as one unit of the suite where the package is marked and as one step
    of the change check where a changed path is the package's or matches its `reads`.
- **A project of its own declares its browser test files** in the same `bmo` section of its own
  manifest, as `browser`: globs from the project's folder, reaching none of a project standing inside it.
  Where the suite marks the project, each file they reach is a unit of its own in the suite's browser
  lane, started once the project's check has ended (`docs/suite.md`, 「Browser files run in a lane of one」).
- **A project of its own** is a directory carrying its own package manifest and the lockfile a declared
  language names (`docs/testbed.md`, 「Languages」) that the root workspace does not list; a
  repository declaring no such language has none to find. Projects nest: a directory carrying the two
  files inside another project is a project of its own, beside the one holding it. The directory
  declares itself by its two files, unless the declaration
  turns discovery off or names its projects (`docs/declaration.md`). It names its own check: the
  `check` script of its manifest, run inside it, its static check and its tests together, readying the
  project first: the testbed itself installs nothing in a project, so the check installs the project's
  locked packages. Its tests are the script's last step, and every step ends the script where it fails,
  so a check whose tests ran passed every step before them and a run once more of the files that
  failed stands for the whole check (`docs/suite.md`, 「What failed runs once more」). A project whose
  manifest gives none fails every run that selects it. The project's
  test files are run by that script and by nothing else of the testbed's, apart from the browser files
  it declares: a script that stops at the static check leaves them run nowhere (`docs/suite.md`, 「Which
  files run」).
- **A tool checked by its manifest** is a folder under the declared tools directory that no crate,
  package or project holds, declared by its own manifest, whose check command runs inside the folder.
  A manifest that does not read, or names no check, fails the unit.
- **The rule, over a set of changed paths:**
  1. **Owner:** a path inside a package's directory marks that package; the longest directory wins.
  2. **Root configuration:** a file at the root that a declared language names as its root
     configuration, such as the workspace manifest, the lockfile and the runner's and compiler's
     configuration, marks every package.
  3. **Reads:** a path matching a package's `reads` marks that package. A path outside every package,
     project and tool that no `reads` matches marks every package, so a file nobody claims is never
     skipped.
  4. **Runs:** a changed path that is a file a package's `runs` names, or a file that file imports at
     any depth, marks that package.
  5. **Dependents:** a package whose dependencies name a package marked by the rules above is marked in
     turn, until nothing more is marked. A mark by `reads` alone stops at the package that declared it.
  6. **Always:** every package declaring `always` is marked.
- **A path of a Cargo workspace marks crates:**
  - a path inside a crate's directory marks that crate;
  - a crate standing at the workspace's root holds every path that no other crate's directory holds and
    no other unit holds: a project of its own, a workspace package, a tool checked by its manifest, a
    folder below the root where a declared language's runner reads its configuration, and a test file a
    declared language runs. Such a path marks it as its own, beside any crate whose `reads` match it;
  - the root `Cargo.toml`, `Cargo.lock` and `rustfmt.toml`, and anything under `.cargo/`, mark every
    crate;
  - a path matching a crate's `reads` marks that crate, and the mark stops there;
  - a crate whose dependencies name a marked crate is marked in turn, read from `cargo metadata`.
- **A path inside a project of its own marks that project** for its own check and no package; so does
  the project's own path where a range moved a submodule's pin. Where projects nest, a path belongs to
  the innermost project whose directory holds it, as a path belongs to the deepest package: a path
  inside the nested project, its manifest and lockfile among them, marks the nested project alone,
  and a path of the outer project outside it marks the outer project alone. A change holding paths of
  both marks both, each for its own check.
- **The changed paths** are the commits of the range the caller names, with every file in the working
  tree that is modified or not yet tracked. With no range the run takes everything.
- **The run says why.** Its first lines name every marked unit and the rule that marked it.
- **An import is followed** where it names a file by a relative path, or by the name of a workspace
  package and a module inside it: a package the root's workspaces list, or one a project of its own lists
  in its own manifest. An alias a configuration defines names nothing followed.
- **What it cannot see:** a launch whose path is assembled from pieces, and an import no static reading
  finds. The daily whole run catches both (`docs/suite.md`).

## The static check

- **The quick checks go first,** where the repository enables any (「The quick checks」).
- **Then the steps are the repository's,** declared in order under `[check]` (`docs/declaration.md`):
  a type check, a format check, a lint, a front-end build. Each runs through `sh` in the directory it
  declares, the root by default, in the lane it declares or where it stands, and is timed. A step
  declaring the paths that reach it runs only for a change touching one of them; a check of every path
  runs them all.
- **A Cargo workspace's check comes last,** where a crate is marked: `cargo fmt --check`, then
  `cargo clippy --all-targets --locked` with every warning an error, both through the Cargo verb, in
  the wide lane, run again where the kernel killed it for want of memory; formatting compiles nothing
  and runs at once. `--locked` holds the lock's registry and git packages as they stand; its path
  packages, a sibling repository's crates among them, are first brought to the versions their manifests
  name (`docs/deploy.md`, 「The Cargo build directory」), so a sibling's new version never fails it.
  - **Both judge the repository's own packages and no other:** the whole workspace where every member
    stands inside the repository's root, and otherwise each member inside it, named. Formatting never
    follows a path dependency, so a sibling repository's crate the workspace names, and the workspace
    that crate belongs to, are never formatted; the lints reach the workspace's members alone, and a
    path dependency is compiled as a dependency, its warnings shown and never denied. A workspace Cargo
    cannot read is judged whole, and one with no member inside the root owes neither step.
  - **A declaration declares no step of its own that type-checks, formats or lints the workspace's
    crates** (`docs/declaration.md`, 「`[[check.step]]`: one static step each, in order」); the rules
    after this one say what becomes of a step a declaration still declares.
  - **A declared step that only repeats it is left out,** so a check formats and lints once, and a
    declared format step never follows a path dependency into a sibling repository's checkout. A step
    only repeats it where it runs at the root and its command, split the way the shell splits it, is
    Cargo's, `cargo` or the Cargo verb `bmo-testbed cargo`, with one of these subcommands and no other
    word:
    - `fmt` with `--check`, given before `--` or alone after it, besides `--all`, `-q` or `--quiet`,
      which the workspace's formatting covers;
    - `clippy` or `check`, besides `--all`, `--workspace`, `--all-targets`, `--locked`, `--offline`,
      `--frozen`, `-q` or `--quiet`, and for `clippy` nothing after `--` but every warning denied, as
      `-D warnings` or `-Dwarnings`, which the workspace's lints cover, since they check every crate as
      they lint it and deny every warning.
  - **Any other word runs the step as declared:** a feature, a package, a target selection, a lint, a
    manifest path, a toolchain, a variable set before the command, a shell operator, and a `fmt` that
    rewrites rather than checks; so does a step declaring another directory. Where no crate is marked,
    or the workspace has no member inside the root, every declared step runs.
  - **A step left out says so in one line,** naming the step and the workspace check's step that covers
    it, and counts as passed: exit 0, no wait and no run, in the steps a caller is told, a deployment's
    progress record and its times.
- **Cargo runs only through `bmo-testbed cargo`,** never plain `cargo`. The verb sets the shared build
  directory (`docs/deploy.md`, 「The Cargo build directory」) so dependencies compile once; a plain
  `cargo` lands in the checkout's own `target/`, which shares nothing and which the prune takes away.
  The harness hook refuses a plain heavy `cargo` and names the verb line to run instead
  (`docs/testbed.md`).
- **Every step runs to its end,** a failing one stopping none after it, and each says its exit code,
  how long it waited for its turn and how long it ran. A step holding a lane's turn that writes nothing to
  its output files for the silence limit is ended as hung and fails with exit code 124 (`docs/testbed.md`,
  「A holder whose command falls silent lets go」), and so does the workspace's own lint step, which holds
  no turn of its own but is watched the same way.
- **A step a signal ended says the signal.** Its line, and the check's failing line where it is the
  first to fail, name the signal in place of an exit code, so a command killed from outside is never
  read as a lint or a test that failed; it counts as exit code 1 for a caller.
- **A step ends when its command ends.** Its output is taken through files of the run's own, never a
  pipe, so a process the step left running, a server a test stood up or a stand-in it never stopped,
  holds nothing the check waits on.

## The quick checks

- **They read the tree without running any of it,** take seconds together, and are the failures a
  check meets most, so they run first. Each runs only where the repository's declaration enables it,
  and reads the paths the declaration names; the testbed ships no word list and no ledger.
- **Recorded input and installed packages are never read:** a file under a `fixtures/` or a
  `node_modules/` folder holds words somebody else wrote. `wording` and `dashes` also pass over a test
  file, a `.test.` file, a file under a `tests/` or `test/` folder or a `tests.rs`, whose words are made
  up; `language` counts its lines as test input.
- **`wording`:** no tracked or new file the declaration's `wording_files` reach, the wording document
  itself aside, uses a term the document bans, and no allowed use in it has gone stale; a file that is
  not UTF-8 text holds no words to read. The document's shape, how a banned term is found on a line of
  its own language and what an allowed use covers are bmo-text's wording (bmo-text's `docs/api.md`,
  `wording`); which files are read is the testbed's.
- **`dashes`:** no text the code writes for a person holds a dash between words. People's text is
  every literal in the files the declaration names as people's text. Comments, regular expressions,
  a Rust test module and a literal that is one dash and nothing else pass. Each language is read by
  its own parser: TypeScript by oxc, a single-file component by the tree-sitter HTML grammar with oxc
  for its scripts, Rust by the tree-sitter Rust grammar, YAML and TOML by their serde readers.
  - **`check dashes <file…>`** rewrites a markdown file's dashes between words the way a person types
    the same sentence, as bmo-text's `without_dashes` rewrites a wrapped text (bmo-text's
    `docs/api.md`, `dashes`).
- **`language`:** no code file holds more lines with a letter that is not English in them than the
  repository's ledger allows it, and a file the ledger does not name is allowed none; a ledger file not
  yet written allows none anywhere. Whether a line holds such a letter, written as itself or named by an
  escape, in a comment as much as in a literal, is bmo-text's verdict (bmo-text's `docs/api.md`,
  `english`): a letter of any script but Latin. A language a file cannot do without, the letters of a
  check that reads them or the input of its tests, is written as itself and stands in the ledger. The
  ledger is a JSON object from each file's path to its count. `--settle` lowers the ledger to what each
  file holds now and never raises it; `--kinds` reads each area's such lines by what holds them: a
  comment, test input, people's text, another literal, or a line of a language no parser here reads.
  `korean` names the same check, in the declaration and as the verb's part, and `korean_ledger` the
  same ledger.
- **`layout`:** the README's folder table, the table whose first header cell is `Folder`, names every
  top-level folder and no other; no folder of the top two levels is named for a language outside a
  project of its own; and every folder holding a `Cargo.toml` or a `package.json` is a member of its
  workspace or a project of its own, and every member holds one.
- **`dependencies`:** every dependency a manifest names is one the declaration allows. A crate's
  manifest is read in every dependency table, a target's included, and a package's in every
  dependency list. A registry crate or package passes, and so does one inside the repository. One
  from another repository, named by a path outside the root, by git, or by a package spec naming no
  registry, passes only where the crate or package it names, a renamed one by its own name, is among
  the declared base libraries, or where it is a dev-dependency on the testbed's test kit: the
  package `@bmo-testbed/testkit`, or the crates `bmo-testbed-test-support` and
  `bmo-testbed-scratch-database` (`docs/boundary.md`, 「The crates」).
- **What `base_libraries` is not:** the declaration's `upstreams` name the repositories whose major
  changes run this repository's suite (`docs/suite.md`, 「The dependents of a major change」); the
  `dependencies` check reads `base_libraries` alone.
- **`cargo-profile`:** the repository's root `Cargo.toml` holds every profile setting that keeps each
  build's output small, as bmo-chat's `docs/conventions.md`, 「Rust builds」, states them: `[profile.dev]`
  with `debug = "line-tables-only"`, `split-debuginfo = "unpacked"` and `incremental = false`,
  `[profile.test]` with `debug = "line-tables-only"`, `strip = "debuginfo"` and `incremental = false`,
  `[profile.dev.package."*"]` and `[profile.test.package."*"]` with `debug = false`, and
  `[profile.release]` with `debug = false` and `strip = "debuginfo"`. A setting left out or set otherwise
  is named with what it should be; a repository with no root `Cargo.toml` has nothing to hold. Setting
  them is each repository's own change. The profiles state what a build typed outside
  the verb keeps; the Cargo verb turns incremental state on over them for every profile
  (`docs/deploy.md`, 「The Cargo build directory」).
- **`personal-data`:** no file of the checkout holds personal data. Every file git tracks or would track
  is read a line at a time for the forms the policy names: a mail address, an account's home path, a telephone
  number, a token and a private network address. A finding names the file, the line and the kind, never
  the value.
  - **Text alone is read.** A file whose bytes are not valid UTF-8, such as a packed release, an image or a
    font, holds no lines and is passed over, so a run of its bytes that happens to take one of the forms'
    shapes is no finding. The count the clean line gives is of the files read.
  - **What stands:** a mail address in the space kept for documents (`example.com`, `.test` and their
    kin); a unit of the service manager written as a template and its instance, and a package named
    with its version, which both have a mailbox's shape; a version pinned after an equals sign, which has
    an address's; a home path whose folder names a role, such as `user`; digits that stand inside a longer number,
    as a fraction's do, and have a telephone number's shape; a folder named `home` or `Users` under
    another folder, which is no account's home; a token written all in lowercase, which is a
    placeholder somebody typed, a generated secret being mixed case; an address of no private network, the
    loopback among them; a whole block kept for private networks, written as its first address and its length; and a finding whose matched text holds a string the declaration's
    `personal_data_allowed` names.
  - **One check for every repository.** The forms and what they let stand are the testbed's policy, so a
    repository keeps no copy of them and no test of its own for this; what differs in one repository is
    its `personal_data_allowed`.
- **`shared-versions`:** the repository's Cargo manifests and lock, and the packages its package manifests
  vendor, hold to the shared versions, as bmo-chat's `docs/conventions.md`, 「Shared versions」, states the
  rule. It reads files alone and runs no Cargo and no package manager.
  - **Where the shared versions stand:** `shared-versions.toml` at the root of the repository the
    declaration's `[check.shared_versions] repository` names, found in the machine's registry of checkouts
    (`docs/suite.md`, 「The dependents of a major change」) as the checkout whose declaration gives that name,
    so a worker's copy and a scratch clone read the same file as the primary checkout. A registry that does
    not read, no checkout or two declaring the name, and a file that does not read are the check's one fault.
  - **The file as that repository's main branch holds it:** the check reads the file from the head of the
    branch that checkout's own declaration names as `[check] main`, as `docs/declaration.md`, 「The main
    branch's head」, reads it without asking the remote, through git, never from the
    checkout's working tree. A shared version is one every consumer is held to only once it is on the
    shared repository's main; a checkout of that repository standing on a release branch or a job's branch,
    or holding the file changed and not committed, is work under way, so whatever it holds there leaves the
    answer where main has it. A checkout that is no git repository, or whose main branch does not hold the
    file, is the check's one fault. The one exception is a placed run handed its checkouts (`docs/boundary.md`,
    「The placed machine」): there the copies are the caller's, which lays the file over its copy as the
    caller's main holds it, so the run reads the file as it stands in the copy. A placed run of a worker's
    copy of the shared repository itself knows that repository by the primary checkout's copy alone, so the
    bench of the worker's copy is no second checkout declaring the name (`docs/boundary.md`, 「The placed
    machine」).
  - **What it leaves out:** a manifest, a lock or a package manifest one of the globs of the declaration's
    `[check.shared_versions] exclude` reaches is not read at all, so a fork's upstream folders, which the
    repository neither builds nor may rewrite, stand outside the check while its own files stay under it.
    The globs read paths from the root, a `**` reaching any depth.
  - **The manifests:** every `Cargo.toml` the checkout tracks or holds new, in its
    `[workspace.dependencies]` and in every dependency table, a target's included. An entry naming a listed
    crate, by its key or by the package it renames, is the file's line byte for byte after its key, its
    comments aside; an entry taken from the workspace adds nothing but `optional`. A fault names the
    manifest, the table and the crate, the entry as it reads and the line to write instead.
  - **The lock:** every `Cargo.lock` the checkout tracks or holds new. A registry package whose name the
    snapshot lists, on a semver line the snapshot names, stands at the snapshot's version; one on another
    line passes. A package read from a path is no registry package and is not read, whatever version the
    lock gives it. A fault names the lock, the crate, both versions and the Cargo verb's `update --precise`
    line that moves it.
  - **The packages:** every `package.json` the checkout tracks or holds new, outside a `node_modules/` or a
    `fixtures/` folder, in each of its lists naming packages: `dependencies`, `devDependencies`,
    `peerDependencies`, `optionalDependencies`, `overrides` and `resolutions`. An entry naming a package the
    file pins under `[packages]`, by its version, its file and the file's sha256, is the pinned release's or
    one of the earlier releases the file's `[compatible]` table lists for that package, each by its version,
    its file and the file's sha256:
    - an entry naming a file, `file:` and a path from the manifest's folder, names one of those releases'
      files, and the bytes standing at that path hash to that release's sha256. A path naming any other
      file, a file not standing there, and bytes that hash otherwise each fail, the last naming both hashes;
    - any other entry is one of those releases' versions as it is written, so a range, a tag or another
      version fails.

    Compatible releases listed for a package the file does not pin leave the file unread.

    The pinned release and the compatible ones are the pin's range. A fault names the manifest, the list,
    the package, the entry as it reads and what to write instead: an entry standing on no release of the
    range is named with the range's versions in version order and the pinned release as the one to vendor.
    The file names no unit, so a package pinned there holds every checkout alike; a file pinning no package
    holds no package manifest.
  - **An exception** the declaration names for a crate (`docs/declaration.md`) holds the crate's entries to
    the exception's line instead of the file's, and lets the lock hold the versions it names. One for a
    crate the file does not list, whose line is the file's or does not read as one line for the crate, or
    naming a version the lock does not hold, is named.
  - **`check shared-versions --report`** holds every checkout of the registry to the same file, each with
    the exceptions and the files left out its own declaration names, whether or not it enables the check,
    and changes nothing: a line per checkout naming the crates whose entries differ, the lock's versions
    that differ, and for each pinned package the checkout names, the entries it writes for it beside the
    pinned version and the pin's range; a line for each checkout whose declaration does not read; and a
    last line counting the checkouts that differ. It ends 1 where any checkout differs.
- **`quick` runs every enabled one** and prints a line for each, ending non-zero where any failed.

## The change check

- **One call for a change's writing check and its first verification:** the `check` verb. In order:
  the static check's own steps for the change, then the test files the change touches, `cargo test` on
  the crates it touches itself, and the check of every tool, project and package the change reaches, a
  step each. The test files are one step for each declared language and folder they run from
  (`docs/suite.md`, 「Which files run」), each file handed by its path from that folder. The whole suite
  stays with the close.
- **Its static steps are the static check's,** asked of the same place with the change it reads, so the
  two never check differently (「The static check」): the enabled quick checks, the declared steps the
  change reaches, and, where the change marks a crate, the Cargo workspace's `cargo fmt --check` and
  `cargo clippy --all-targets --locked` with every warning an error over the repository's own packages,
  through the Cargo verb in the shared build directory.
  - **The lints are its type check:** they check every crate as they lint it, so no `cargo check` runs
    beside them.
  - **A declared step that only repeats them is left out** as the static check leaves it out: its line
    is an `ok` line saying it was left out and which of the workspace check's steps covers it, with no
    wait and no run, and it counts as passed.
- **The change is read two ways.** Given files, it is those files. Given none, it is every path changed
  since `--since`, by default the merge base with the main branch, with the working tree and the
  untracked files.
- **What it touches is one function's answer:** a crate a changed path belongs to or declares it reads
  is tested, and it and every crate depending on it are checked; a test file runs where it changed
  itself or loads a changed file through its imports.
- **A crate whose only changed paths are its test files runs those tests alone,** a step of its own for
  each test target they reach, read from `cargo metadata`; any other path of the crate, or one its `reads`
  match, runs the crate's tests whole. Both layouts of a crate's tests are read the same way:
  - a changed file that is a test target's own source runs that target whole, `--test <target>`, and
    so does a change to a harness's `main.rs`;
  - a changed `.rs` file in the folder of a harness, a target whose source is a `main.rs`, is that
    harness's module named by the file, or by the folder it stands in below the harness's folder, and runs
    by the module's name filter, `--test <target> -- <module>::`, the filters of one target's modules in one
    step; where several harnesses' folders hold it, the deepest one's;
  - a module whose files carry no test attribute, `#[test]` or one ending in `::test`, or that another
    file under the crate's `tests/` names by a path through it or by a `#[path]` attribute, is shared, and
    runs the crate's tests whole; so does a changed file under `tests/` that is neither, such as a
    fixture or a helper of per-file targets.
- **Every step runs to its end**, and the heavy ones wait for the testbed's turn. Every step, a declared
  one that runs where it stands among them, is watched for silence and ended as hung once it writes nothing
  for the silence limit.
- **While a step runs it says so** on standard error, once a minute by the policy: the step's name and how
  long it has run. A step's own output is taken into its log, and a step waiting for its turn or run on the
  placed machine writes next to nothing here, so without the line the check would stand silent for as long
  as the step lasts, and a caller watching its output could not tell it from a hung one. The line says
  nothing of the step's health, which is why every step is watched for silence: a silent step is ended by
  the check itself, never left to a watcher above it, which hears the check speak all the while.
- **All the output goes to one log,** one per checkout under the data root's `testbed/` unless `--log`
  names another, written afresh by every run. What it prints is a line per step with its outcome and times,
  and the tests' totals and failures as the `log` verb reads them where test files ran.
- **Under a failing step stands what failed,** read out of the step's own output by every language at once,
  for a crate's tests, a project's or a tool's check and a static step alike: its totals where a language
  reads them, then what the languages name, each failed test with the lines its panic left under it and each
  compiler error with its place; where no language names anything, its last lines. As many lines stand as
  the policy names, and those left to the log are counted in a line of their own. The failing test files'
  own step is the `log` verb's to name.
- **A failing check's log is kept apart** beside the log, named as the log is with `failed` before its
  extension, so the next run of the checkout, which writes the log afresh, leaves it; only the next failing
  run replaces it. The check says where it is kept on a line before the log's own last line. A placed change
  check keeps it on the machine it ran on, and its lines, which come back, carry what failed and that path.
- **It ends 1 where any step failed, 2 where the change could not be read.**

## The change check over several checkouts

- **One call checks a change that spans several repositories:** the `check` verb given a repeated
  `--checkout`, each naming a checkout by its path, read from the directory the verb runs in, and
  optionally the commit its change is read since, written after the path and an `@`: `<path>@<rev>`. A
  word naming a directory that stands is a path with no since; any other word is split at its first `@`,
  so a revision holding an `@` of its own, such as `HEAD@{1}`, keeps it. A checkout given no since reads
  its change since its merge base with the main branch its own declaration names, as the check of one
  checkout does. With no `--checkout` the change check is the one above, over the checkout the verb runs
  in, and `--checkout` is never given with `--since` or with files.
- **Every checkout is read before anything runs:** its top level as git names it, its own declaration, and
  its since resolved to the commit itself. A path that names no checkout, two words naming one checkout,
  a since that names no commit, and a since that is not an ancestor of the checkout's `HEAD`, which a
  placed bench could never read since it holds the history of `HEAD` alone, end the call 2 with a line
  naming the word, and nothing runs.
- **Each checkout is planned by its own declaration,** exactly as the check of one checkout is planned
  (「The change check」): its quick checks, its declared steps, the formatting and lints of its own Cargo
  workspace, the test files, crates, projects and tools its change touches. Nothing is planned across them:
  what one checkout's change touches in another is that other checkout's change only where it is changed
  there too.
- **The set runs in one wide seat:** the call takes one turn of the wide lane for the whole set and runs
  every step of every checkout inside it, the checkouts one after another in the order given, each step
  of one checkout run in that checkout. The steps that take a turn of their own, the test files and the
  Cargo verb among them, run inside the set's turn and never wait in the line again. While the turn is
  waited for, the line says where the call stands every notice; once it came, a line says how many
  checkouts the one seat holds and what the turn came to; while a step runs, its line every minute, as for
  one checkout, so the call is never silent while it lasts.
- **Where it runs is the placed machine alone** (`docs/boundary.md`, 「Placement」, `combined_order`): the
  whole set goes there as one run, every checkout laid in one bench at the place it has under this
  machine's home, so the checkouts stand there as they stand to one another here and a path one names to
  another resolves there as it does here. It waits in that machine's line however long that takes, and
  runs on this machine only where the placed machine's settings name this machine in `combined_order`.
- **Every line of a checkout's check names its repository:** the heading of its change, each step's line
  as the step ends, printed as it ends, what failed under a failing step, the line a running step says every
  minute, and its own log's line, each opening with the name of the repository its declaration gives. The
  set's own lines name none: where its turn stands, the last lines below, and what the placement says.
  Each checkout writes its own log as the check of one checkout does, and
  the set writes one log holding every checkout's log in turn, under the data root's `testbed/` named for
  the set unless `--log` names another.
- **The last lines name what failed:** a line for each repository with a failed step, naming the
  repository and every step of it that failed; then, where any failed, the set's log kept apart as one
  checkout's is, on its `failing log kept:` line; and last the set's `log:` line.
- **Each repository is one test run in the record** of the machine the steps ran on (`docs/boundary.md`,
  「The record」): kind `check`, its checkout, its repository's name and its commit, begun once the set's
  turn came and ended with the counts its own test files and crates came to.
- **It ends 0 where every step of every checkout passed;** 1 where any failed, or where the placement
  refuses to run the set here; 2 where a checkout could not be read; and 75 where the set was not run: its
  turn never came, or the placed machine could not be reached or brought to the live release.

## Imports a move left behind

- **The `imports` verb** finds every relative import that names nothing, in the files named or in
  every code file the checkout tracks or holds untracked: a static or type import, an `export from`, a
  bare import, a dynamic import, a `require()` and a test's module mock. Only an import standing in
  code is one, read off the parsed source.
- **Where a broken one goes:** to the one file whose path ends the way the import's path ends, tried
  whole first and then one leading directory shorter at a time; the first length that matches anything
  decides.
- **How it is spelled:** relative, where the file is in the same workspace package; the package's name
  and the module's path inside it, where it is in another. An extension the import spelled is kept.
- **It refuses rather than guesses.** An import whose tail more than one file ends with, or nothing
  ends with, is named and not rewritten.
- **Without `--fix` it only says,** a line per broken import, and ends non-zero while any is there.
  **`--fix` writes, all or nothing:** where any import is refused, nothing is written.

## The test count

- **A change may not remove a test unnoticed.** The `count` verb counts the tests a range removed over
  the test files it changed. A test is one call of `test` or `it`, known by its file and its title,
  read off the source by oxc's parser; a test file inside a submodule is read at the commit each end
  of the range pins it to.
- **What is not a removal:** a title that stands in another test file at the range's end, a moved test;
  and as many of a file's removed titles as the file gained new ones, rewritten in place.
- **What accounts for a removal:**
  - a Rust test that replaces it, naming the removed title word for word in a tracked `.rs` file;
  - a commit of the range carrying a trailer line `Retired-test: <title> — <reason>`, the reason saying
    why the behaviour is no longer anyone's. The line's last dash parts the two.
- **A removal nothing accounts for fails,** naming each title and its file, exit code 1.
