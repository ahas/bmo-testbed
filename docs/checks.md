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
    tests start;
  - `always`: the package is selected on every change;
  - `check`: a command checking a part of the package that is not covered by its runner, run inside the
    package's folder through `sh`, as one unit of the suite where the package is marked and as one step
    of the change check where a changed path is the package's or matches its `reads`.
- **A project of its own** is a directory carrying its own package manifest and lockfile that the root
  workspace does not list. The directory declares itself by its two files, unless the declaration
  turns discovery off or names its projects (`docs/declaration.md`). It names its own check: the
  `check` script of its manifest, run inside it, its static check and its tests together. A project
  whose manifest gives none fails every run that selects it.
- **A tool checked by its manifest** is a folder under the declared tools directory that no crate,
  package or project holds, declared by its own manifest, whose check command runs inside the folder.
  A manifest that does not read, or names no check, fails the unit.
- **The rule, over a set of changed paths:**
  1. **Owner:** a path inside a package's directory marks that package; the longest directory wins.
  2. **Root configuration:** the root workspace manifest, the lockfile and the root runner and
     compiler configuration mark every package.
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
  - the root `Cargo.toml`, `Cargo.lock` and `rustfmt.toml`, and anything under `.cargo/`, mark every
    crate;
  - a path matching a crate's `reads` marks that crate, and the mark stops there;
  - a crate whose dependencies name a marked crate is marked in turn, read from `cargo metadata`.
- **A path inside a project of its own marks that project** for its own check and no package; so does
  the project's own path where a range moved a submodule's pin.
- **The changed paths** are the commits of the range the caller names, with every file in the working
  tree that is modified or not yet tracked. With no range the run takes everything.
- **The run says why.** Its first lines name every marked unit and the rule that marked it.
- **What it cannot see:** a launch whose path is assembled from pieces, and an import no static reading
  finds. The daily whole run catches both (`docs/suite.md`).

## The static check

- **The steps are the repository's,** declared in order under `[check]` (`docs/declaration.md`): a
  type check, a format check, a lint, a front-end build. Each runs from the repository's root through
  `sh`, in the lane it declares or where it stands, and is timed.
- **The quick checks go first,** the ones the repository enables (「The quick checks」).
- **A Cargo workspace's check** runs where a crate is marked: `cargo fmt --check`, then `cargo clippy
  --workspace --all-targets --locked` with every warning an error, both through the Cargo verb, in the
  wide lane, run again where the kernel killed it for want of memory; formatting compiles nothing and
  runs at once.
- **Every step runs to its end,** a failing one stopping none after it, and each says its exit code,
  how long it waited for its turn and how long it ran.

## The quick checks

- **They read the tree without running any of it,** take seconds together, and are the failures a
  check meets most, so they run first. Each runs only where the repository's declaration enables it,
  and reads the paths the declaration names; the testbed ships no word list and no ledger.
- **`wording`:** no tracked or new file the check covers uses a term the repository's wording document
  bans, and no allowed use in it has gone stale.
- **`dashes`:** no text the code writes for a person holds a dash between words. People's text is
  every literal in the files the declaration names as people's text. Comments, regular expressions,
  test files and a literal that is one dash and nothing else pass. Each language is read by its own
  parser: TypeScript by oxc, a single-file component by the tree-sitter HTML grammar with oxc for its
  scripts, Rust by the tree-sitter Rust grammar, YAML and TOML by their serde readers.
- **`korean`:** no code file holds more lines with Korean in them than the repository's ledger allows
  it, and a file the ledger does not name is allowed none. `--settle` lowers the ledger to what each
  file holds now and never raises it; `--kinds` reads each area's Korean lines by what holds them.
- **`layout`:** the README's folder table names every top-level folder, no folder of the top two levels
  is named for a language, and every manifest's folder is a member of its workspace or a project of
  its own.
- **`dependencies`:** every dependency a manifest names is one the declaration allows: a registry crate
  or package, or one of the declared base libraries, and a dev-dependency on the test kit.
- **`quick` runs every enabled one** and prints a line for each, ending non-zero where any failed.

## The change check

- **One call for a change's writing check and its first verification:** the `check` verb. In order:
  the enabled quick checks, the declared static steps the change reaches, `cargo check` on every crate
  the change marks, then the test files the change touches, `cargo test` on the crates it touches
  itself, and the check of every tool, project and package the change reaches, a step each. The whole
  suite stays with the close.
- **The change is read two ways.** Given files, it is those files. Given none, it is every path changed
  since `--since`, by default the merge base with the main branch, with the working tree and the
  untracked files.
- **What it touches is one function's answer:** a crate a changed path belongs to or declares it reads
  is tested, and it and every crate depending on it are checked; a test file runs where it changed
  itself or loads a changed file through its imports.
- **Every step runs to its end**, and the heavy ones wait for the testbed's turn.
- **All the output goes to one log,** one per checkout under the data root's `testbed/` unless `--log`
  names another, written afresh by every run. What it prints is a line per step with its outcome and times,
  the first lines a failing static step printed, and the tests' totals and failures.
- **It ends 1 where any step failed, 2 where the change could not be read.**

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
