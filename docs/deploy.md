# Deployment: a repository's releases on the machine it runs on

## What it is

- **The `deploy` verb puts one commit of a repository live** on the machine it runs on, as a release
  beside the one serving, flipped onto in one rename, with its declared services restarted and
  watched, and rolled back where they do not come up. What the repository builds, which services it
  runs and how its health is read are its declaration's (`docs/declaration.md`).
- **The hosted deploy tool** puts a commit live for a session the verb refuses, through the tool host
  (「The hosted deploy tool」).
- **The `backups` verb** sends the backups deployments took to remote storage and keeps them rotated.
  A machine's runtime data apart from any deployment is the data backup's (`docs/backup.md`).
- **One repository, one deploy tree.** Every place below is the declaration's, relative to the
  repository's declared deploy root.

## The places

| Name | What it is |
|---|---|
| Deploy tree | A symbolic link to the live release: the one stable path every unit, every link and every caller names, moved from one release to the next by a single rename |
| Releases | `deploy-releases/` beside the link, one release per deployed commit, named by its first twelve hex digits |
| Build tree | `deploy-expand` beside the link, a detached worktree moved to each commit, where the release build runs, its target directory kept warm between deployments |
| Data root | The repository's runtime data, inside no checkout and inside no release |
| Deployment record | `deployed.json` under the deploy root, naming the commit that runs, its subject, the reason it was deployed, the release's version, when it was laid down and the backup behind it |
| Deployment log | `deployed.log` under the deploy root, one line per deployment |
| Progress record | `deploy-progress.json` under the deploy root, where a run stands while it runs |
| Backup directory | `<deploy root>/backups` unless the declaration names another, where the deployment's backup is written, readable by the user alone, inside neither the data root nor a git work tree |

- **The data root is never inside a git work tree.** A deployment finding it there does not start.
- **A deployment's own files stand under its deploy root, never in its data root.** The data root
  holds the repository's runtime data alone, so several repositories may declare the same data root
  without one deployment writing over another's record or log.
- **Every place but the deploy tree can be moved** by the variables `docs/boundary.md` lists. The
  deploy tree is the declaration's alone: every process a release starts carries `BMO_DEPLOY_DIR`
  naming its own release, so a deployment started from any of them would otherwise aim at that
  release's repository rather than its own.

## The releases

- **A release is a detached git worktree of the repository at its commit,** set up the way any
  checkout of it is: its submodules brought to the commits it pins, then the declared setup steps, then
  the declared env links. Its `bin/` holds the binaries built for that commit, and its
  `bin/built-from` names that same commit.
- **The trees the workspace names by a path outside the repository — the base libraries and the
  sibling repositories' crates a release build reads — are laid where each manifest names them, as
  links to the checkout's own trees.** A worktree of the repository alone carries nothing beside it,
  so the release and the build tree read such a tree from the workspace the checkout stands in; the
  build tree's declared path reaches the deploy root's parent, where the repositories of one
  workspace share one. A tree that is a member of another Cargo workspace — one Cargo, asked from the
  checkout's copy of it, places under a workspace root above it — reads its inherited fields from
  that root, so the link is laid at the root instead, and the named path resolves through it; every
  tree named inside one workspace shares that one link. A standalone package is linked by itself. The
  list of trees is read from the checkout's workspace, since a worktree reads no manifest until the
  trees its manifests name stand where they point. A tree the checkout does not carry refuses the
  deployment before anything is built. Where a link is to be laid, a link already on the checkout's
  tree is left as it is, a link pointing elsewhere is moved onto it, and a folder holding nothing but folders and links, as an earlier laying
  leaves, is replaced; any other tree standing there is left as it is.
- **A release is finished before anything serves from it.** Prepared and built while the live release
  goes on serving; the deploy tree's link is then moved onto it by a new link written beside the old
  one and renamed over it, so every reader sees one release or the other and never a tree half moved.
- **The build tree is stamped where its commit moved.** A build reads a source by its modification
  time against what it built before, and the build tree is kept warm between deployments. Every path
  the new commit changed is therefore stamped with the moment the tree is put at it, so a build the
  commit moves is a build that runs; a path the commit removed is none to stamp.
- **A binary is placed into a release once.** A binary whose bytes equal the previous release's is
  hard-linked to that file; a binary that differs is copied in. What differs is what the deployment
  restarts (「The unit links」).
- **A process stays on the release it started from.** Its executable and working directory resolve to
  the release's real path, so a flip changes what the next start runs and nothing a running process
  reads. A process hands the release it reads on to what it starts, through `BMO_DEPLOY_DIR`.
- **Retention:** each deployment prunes the releases twice, before it prepares its own and once it is
  live. A prune keeps the live release, the one before it, the one the deployment itself runs from,
  the one it lays down and every release a live process stands in, and takes every other away through
  the guarded removal and then `git worktree prune`.
  - **A process stands in a release** where its working directory or executable lies inside it, or its
    `BMO_DEPLOY_DIR` names it, as `/proc` says of every process of the account.
  - **More than twelve releases left standing** after a prune sends a `releases-pinned` event naming,
    for each release kept only because a process stands in it, the processes that do.
- **A release is never cleaned.** Its untracked files belong to the runtime. A release goes only whole.

## A deploy tree that is still a directory

- **The deployment's first step adopts it,** before the backup: a deploy tree that is a real directory
  rather than a link is taken into the release layout once.
  1. A release is prepared at the commit the directory stands on, and the directory's `bin/` is
     hard-linked into it, so the binaries running now and the release's are the same files.
  2. The directory and a link onto the release are exchanged in one rename (`renameat2` with
     `RENAME_EXCHANGE`), so there is no instant without a deploy tree.
  3. The directory becomes a retired tree, `deploy.retired-<time>` beside the link, and
     `git worktree repair` points the repository's record of that worktree at it.
- **A run cut off on the way is finished by the next.**
- **The retired tree stays until nothing stands in it,** and the first deployment that finds no process
  in it removes it.
- **A deploy tree that cannot be adopted,** a directory that is not a git worktree at a commit, or a
  file in its place, is refused before the backup.

## The env files

- **Secrets are never kept in two copies.** Each declared env link is a symbolic link in the release to
  a file under the data root, so the deployment's backup carries them along.
- **A missing env file refuses the deployment.**

## The unit links

- **The installed user units the repository's services run through are symbolic links into the deploy
  tree,** under `~/.config/systemd/user/` or `$XDG_CONFIG_HOME/systemd/user/`, naming the stable path
  and never a release, so the flip moves every unit with it and a rollback's flip moves them back.
- **A unit the deployment keeps installed says so itself,** by one line in its `[Install]` section:
  `X-BMO-Deploy=` and its mark. systemd ignores keys beginning `X-`, and no list of such units is kept
  anywhere else.
  - `bot`: the one service whose health the deployment waits on. Linked and enabled, stopped,
    restarted and watched by every deployment.
  - `restart`: a service linked where no unit of its name is installed, enabled, and restarted by every
    deployment just before the `bot` service, in the order of their paths.
  - `binary`: a service that runs one of the release's binaries; linked and enabled, and restarted only
    where this deployment placed different bytes for the program its `ExecStart` names, or linked the
    unit itself.
  - `timer`: a timer, with the service of its own name beside it, linked, the timer enabled and
    started.
  - `link`: a unit something else starts, linked and nothing more.
  - `retire`: a unit that goes; once the `bot` service has come up, it is stopped,
    disabled and unlinked, a timer's service with it.
- **Before anything changes, every unit link into the deploy tree is read,** and the commit asked
  whether it has the file it points at. A link the commit would leave dangling is matched to the one
  tracked file of the commit with the unit's own name and relinked to it right after the flip. A link
  with no such file, or with more than one, is refused before the backup.
- **A marked unit installed as a plain file** is moved aside as `<unit>.before-deploy`, never deleted,
  and linked in its place; one linked anywhere but the deploy tree is relinked to it.
- **A repository with no `bot` unit** deploys all the same: it restarts what its marks say and waits on
  no health.

## Where a deployment runs

- **Every deploy run runs in a transient user unit of its own,** `bmo-deploy-<stamp>`. A run that is
  not already inside a deployment's unit starts one, prints its name and the command that reads its
  journal, and exits 0; the caller's death reaches nothing from then on.
- **A unit that cannot be started is a refusal.**
- **What runs inside a deployment's unit runs where it is:** the unit is started with
  `BMO_DEPLOY_UNIT` naming it, and every process in it inherits the variable.
- **The unit is handed `PATH` and the roots the caller resolved,** so the run aims at exactly what the
  caller aimed at, and `BMO_TESTBED_FRONT=1`, so every heavy command of the run stands at the front of
  its lane.
- **A dry run and a refusal stay in the caller's process:** a refusal exits 2 there, and `--dry-run`
  says there what the run would do, changing nothing and starting no unit.

## The hosted deploy tool

- **A session the verb refuses puts a commit live through the tool host.** A harness's session carries
  `CLAUDE_PID`, so `deploy` refuses it (「What it refuses」). The testbed therefore provides a hosted
  tool, which the tool host runs outside every harness (bmo-pocket's `docs/mcp-host.md`, 「Placed
  tools」), and a session calls it through `bmo use`, waiting for its answer, or with `--async` going on
  and woken by it once the deployment has ended.
- **Its full name is `testbed/deploy`:** the namespace `testbed` and the entry `deploy`, shown only to a
  session whose record holds the capability `deploy`. The entry acts, so the toolbook reaches it by
  `act`. A call waits for its deployment however long it takes; its manifest names no span.
- **Its arguments,** stated by the input schema beside its manifest:

  | Argument | Type | Meaning |
  |---|---|---|
  | `repository` | string, required | The repository to deploy, by the `name` its checkout's declaration names, with or without a leading `bmo-` |
  | `commit` | string, required | The commit to put live, by its hexadecimal name, whole or shortened; it must be on the remote |
  | `why` | string, required | Why the deploy is made, in English, handed to the verb as `--message` |
  | `dry_run` | boolean, optional | Say what the deployment would do, changing nothing and starting no unit |

- **The checkout comes from the machine's registry** (`docs/suite.md`, 「The dependents of a major
  change」): the file `BMO_TESTBED_CHECKOUTS` names, or `checkouts.toml` under the `testbed/` of the
  data root `BMO_DATA_DIR` names, which the tool host hands every call from the session's record. Each
  checkout the registry lists has its declaration read, and the one whose `name` is the repository asked
  for, a leading `bmo-` taken off both, is the checkout.
- **It runs the verb in that checkout** with the commit, the reason and `--dry-run` where asked, its
  environment the call's with `CLAUDE_PID` and `BMO_DEPLOY_UNIT` taken out, so the verb starts its unit
  as it does for any caller outside a harness. The binary is the one of the release the tool is placed
  from, its `bin/bmo-testbed`, or `bmo-testbed` on the path where the tool runs from anywhere else.
- **It waits the deployment out:** it reads the unit's name off the line the verb prints, asks the
  service manager every few seconds whether the unit is still active, and once it is not, reads the
  deployment record under the repository's deploy root, however long the unit runs.
- **What it answers:**
  - **live:** where the record names the commit asked for and was written after the call began, a first
    line `live:` with the commit and its subject, then the commit live before it, the reason and the
    unit;
  - **not live:** otherwise, an error answer naming the unit and what the record names, with the last
    thirty lines of the unit's journal;
  - **a refusal of the verb,** which ends 2, a verb that ends with any other failure, and a verb that
    names no unit: an error answer carrying what the verb said;
  - **a dry run:** what the verb said.
- **What it refuses,** as an error answer before the verb runs:
  - arguments that are not its shape, a required one empty, or a commit that is not a hexadecimal name;
  - a registry that does not stand, or does not read, named by its path;
  - a repository no checkout of the registry declares, naming the names the registry's checkouts
    declare and the checkouts whose declaration does not read;
  - a repository two checkouts declare, naming both;
  - a repository whose declaration deploys nothing.
- **The testbed's own deployment places it,** before the flip: the tool's executable, its manifest and
  its input schema are copied into the release's `tools/deploy/.placed/`, which is put into the tool
  host's registry for the service `testbed` with the capability `deploy` allowed.

## What the deploy run does, in order

1. **Check the data root and the backup directory,** and stop where either stands where it may not
   (「What it refuses」), or a declared env file is missing.
2. **Pick the commit.** After `git fetch`, take the commit `--commit` names, or the checkout's `HEAD`,
   and check it is on the remote. Adopt a deploy tree that is still a directory. Check every unit link
   will still find its unit file.
3. **Pass the gate:** the declared static check and the suite over the range from the deployed commit,
   holding every lane at the front (`docs/testbed.md`), with what failed run once more. `--no-check`
   leaves it out where the caller already ran it, and so does a declaration with `gate = false`. The
   deployment asks the gate through a hand of its own, which the binary answers with the suite's
   static check and run, so a test stands in for it.
   - **A major change runs its consumer's suites as well.** The major of the commit's `VERSION` is
     compared with the version in the deployment record of the release live before it; where they
     differ, the deployment runs `dependents --run` (`docs/suite.md`, 「The dependents of a major
     change」) after the gate and before the backup. A consumer's failure stops the deployment with
     nothing changed. `--no-check` leaves the gate out but not this run.
4. **Take the backup,** before anything changes (「Backups」).
5. **Prepare the release,** pruning the releases first. A release of that commit still standing whose
   `bin/built-from` names it is taken as it stands; otherwise it is made afresh.
6. **Build,** for a release made afresh: the declared `[build]` steps in the build tree, then the
   declared binaries placed into the release's `bin/`, hard-linked or copied, the ones that differ
   noted. `bin/built-from` is written last. A taken release is compared with the live one instead. A
   build that fails stops the deployment with every service untouched.
7. **The steps before the flip,** declared under `[deploy]` with that phase: each runs in the release;
   one that fails stops the deployment with nothing flipped.
8. **Wait for quiet,** where the declaration names a `quiet` command: run from the release, it answers 0
   where no work stands under way that a restart would cut. Where it answers anything else, the line
   says so with what the command named, and it is asked again every few seconds (the testbed's policy)
   until it answers 0, however long the work takes. A deployment asked with `--force-live` does not wait:
   it says the work it cuts and goes on at once. This is the last step before the record, so the moment it
   answers 0 is seconds before the services stop.
9. **Record it** in `deployed.json` and `deployed.log` under the deploy root. The record names, beside
   the commit, its subject, when it was laid down and the backup, the reason the deploy was made and
   the release's version: the reason is `--message` where the caller gave one, else the commit's
   subject; the version is the commit's `VERSION` file content, empty where it carries none. A
   relaunched session reads the reason and the version to say which unit was deployed and why.
10. **Flip the deploy tree:** confirm every unit link will resolve in the release, write a new link
   beside the deploy tree and rename it over the old one.
11. **Restart:** relink the dangling unit links, link the marked units, reload, restart the `restart`
    units and the `binary` units whose binaries changed, then the `bot` service.
    - With `--before-start <command>`, stop the `bot` service, run the command, then start it.
    - Wait until the declared health answers, however long that takes, or until the `bot` service gives
      up starting: systemd holds it failed or inactive, or has restarted it since the wait began, which
      says the process the restart started has ended. Then read the unit once: up is active and running.
      A `bot` service with no health declared is up where that reading finds it running.
    - A unit that will not restart, one that gives up starting before its health answers, or one not
      read running once it came up is rolled back (「Rollback」).
    - Then the `retire` units go.
12. **The steps after the health,** declared with that phase, in order, each run until it ends. One that
    fails is a warning in the deploy output and never fails the deployment; the next deployment runs it
    again, so a step is written to do only what is due.
13. **Prune the releases.** A release that cannot be removed is a warning.
14. **Start the backup upload,** where the declaration names a remote, as a transient user unit of its
    own, `bmo-backup-upload-<stamp>`, running the `backups` verb, and end without waiting for it.

- **The deployment ends** with a `deployed` event naming the commit, the flaky tests its gate found, and
  the warnings.

## What a deployment says of itself as it runs

- **Every run writes where it stands into one progress record,** `deploy-progress.json` under the
  deploy root, or the file `BMO_DEPLOY_PROGRESS` names. A reader needs only the file and the run's
  process.
- **It names from its start every stage the run means to pass,** its plan: `check`, `backup`, `tree`,
  `build`, `before`, `restart`, `stay-up`, `after`; `rollback` is never planned. Each stage is written
  with when it began.
- **Each heavy step is written as it begins and as it ends,** with its exit code, how long it waited
  for its turn and how long it ran, under the stage it ended in. Steps are named by their declared
  English ids.
- **It ends with an outcome:** `live`, `rolled-back`, `failed`, or `stopped` where nothing went live by
  the run's own judgement, with the reason beside it.
- **A run that dies leaves the record unended.** Its process gone, a reader takes it as cut off; the
  next run writes over it.
- **The testbed's record holds the same,** written with every change of the progress record as the
  deployment the HTTP service answers, with every line the run says as one of its log lines, and,
  before every unit the run stops or restarts, a restart naming the unit and why, for the unit to read
  as it starts again (`docs/boundary.md`, 「The record」).

## A deployment cut off in the middle

- **`deployed.json` then names a commit the `bot` service has not started on.** The record is written
  once the release is built, just before the flip, so the deploy tree stands on the new release or the
  one before, while the service is still the one that started before, or is down.
- **A cut before the flip leaves the live release serving;** the half-made release is made afresh by the
  next run of its commit.
- **The remedy:** `deploy --commit <that commit>` takes the release still standing, flips onto it again
  and starts the service on it.

## Rollback

- **When:** a unit cannot be started or restarted, the `bot` service gives up starting before its health
  answers, or it is not read running once it came up.
- **What the deploy output says first,** so the reason survives the rollback: what was waited for and
  what the last ask heard; the unit as systemd holds it, its state, its main process, its restarts and
  last exit status; the machine's load and its pressure stalls; the tail of the service's journal since
  the restart.
- **The deployment rolls back to the release that was live before,** at once:
  1. stops the service that did not come up;
  2. flips the deploy tree back onto that release, which a deployment never prunes, and relinks the
     dangling unit links, with no setup and no build;
  3. writes that commit back as the deployment record, with a line in the log saying it was a rollback;
  4. reloads the units and restarts the service, with the `binary` units whose binaries differ between
     the two releases;
  5. waits for the health again and reads the unit the same way.
- **The release tree and the service manager are reached through two hands,** one for the tree and one
  for the services, so a test drives a rollback with stand-ins for both.
- **It moves the code alone.** Data the failed release wrote stays; the backup the deployment took
  before anything changed stays for the case where that is not tolerable.
- **What is not rolled back:** a first deployment, a deployment with `--before-start`, and a release
  that no longer stands or a flip that fails, where the service is left down and the event says so.
- **It sends a `deploy-rolled-back` event** naming the commit that failed, why, the commit rolled back
  to, whether the service answers now and the journal to read, and the deployment still fails with a
  `deploy-failed` event.
- **A rollback by hand** is an ordinary deployment of the older commit.

## What it refuses

- **A harness's shell.** A process a harness started, `CLAUDE_PID` set, is refused before anything is
  done. A harness's session deploys through the hosted tool (「The hosted deploy tool」).
- **A checkout with uncommitted changes to tracked files,** where the run takes the checkout's own
  `HEAD`; a run handed `--commit` reads nothing of the open files.
- **A commit the remote has never seen.** A commit only on this machine can never be fetched again if
  the disk dies.
- **A unit link the commit would leave dangling** with no single file of its name to relink to.
- **A data root inside a git work tree,** and **a missing env file.**
- **A backup directory inside the data root or inside a git work tree.** Every file the backup writes
  and takes away again, its staging, its scratch, a part file and a backup that failed, goes through
  the guarded removal, which nothing inside the data root passes.
- **Any deletion inside or above the data root.** Every deletion the deployment makes goes through one
  guarded removal that refuses both directions.

## Backups

- **The backup is taken by the deployment, before anything changes,** into the backup directory:
  `<name>-<date>-<time>.tar.zst`, the name declared, a two-digit ordinal added where two are taken in
  one second.
- **What goes in:** the declared snapshot command's output first, the copy and its manifest the command
  writes into a scratch directory, held in the archive as `.backup-snapshot/`; then the data root packed
  as it is, the declared exclusions left out.
- **How it is compressed:** one tar stream through `zstd -T0` at level 3. A file that changes while tar
  reads it goes in as it stood, which tar ends with exit code 1, a backup taken; any other tar failure
  fails the deployment with nothing changed.
- **Read back as it is packed, once:** every byte written is also hashed and decompressed through a
  second zstd and tar, which take the snapshot out again; the file is then read back from disk and
  hashed again, and the snapshot held to its manifest by the declared verify command. Any mismatch
  deletes that backup and fails the deployment.
- **Each part is timed apart** and written as a step of the progress record: `snapshot`, `pack`,
  `check`.
- **The backup directory is readable by the user alone,** since it holds env files.

## Sending backups away

- **The `backups` verb,** started by every deployment as its last step in a unit of its own, and run by
  hand the same way.
- **One at a time:** a run that finds another going waits for it to end, however long it takes, saying
  so every thirty seconds, then works on what it left.
- **What it sends:** every backup in the directory not yet confirmed, newest first, unless as many newer
  ones as are kept already stand remotely.
- **A copy is confirmed only once read back.** The local file's SHA-256 is taken; it is uploaded with
  that checksum as the object's `sha256` metadata, unless an object of its name already stands and
  reads back matching; the object is read back whole and its size and SHA-256 must match; a match
  writes a receipt beside the backup, `<name>.sent`, holding the checksum, and a mismatch removes the
  object.
- **Then the rotation:** remotely, every object past the newest kept, five unless declared, is removed;
  locally, every backup but the newest is removed with its receipt, or where as many newer copies as
  are kept stand remotely.
- **A failure never touches the deployment,** which ended before the upload began. A step that fails
  leaves its backup on the local disk, the run goes on with the others, and a `backup-upload-failed`
  event names each backup, the step and what the command said.
- **Remote storage** is an S3-compatible bucket reached through the AWS command with a profile, both
  declared.

## The Cargo build directory

- **One build directory is shared** by every checkout of every repository that builds through the Cargo
  verb, `~/.local/share/bmo/cargo-target` unless `BMO_CARGO_TARGET_DIR` names another, so dependencies
  compile once rather than once per checkout. Each checkout's own crates build under names of their
  own, and its target directory under `checkouts/` holds what its builds hand back.
- **Every build goes through the Cargo verb,** which sets the build directory for whatever checkout it
  runs in, a scratch clone as much as a repository of the umbrella. A build typed outside the verb,
  plain `cargo`, lands in the checkout's own `target/` and shares nothing; such a `target/` is a
  stray the prune takes away.
- **Removing what a build superseded.** After a build, check, lint, test, bench or run the Cargo verb
  carried has passed, the units of the checkout's own crates that its earlier builds left and this one
  replaced go. What a build stands on is Cargo's own word, its JSON messages. A record in the
  checkout's target directory names, for each shape of build, the unit its last build stood on at each
  place, with the generation it was built in. The removal takes Cargo's build lock only where no build
  holds it at that moment, leaving what it found to the next build otherwise; it stands in no queue of
  its own, and never fails its build.
- **The prune,** the `prune` verb, run every six hours by `bmo-cargo-prune.timer`, holding Cargo's own
  build lock in every profile directory and standing at the front of each lock's line, never in the
  testbed's:
  - a unit of a checkout's own crates no live checkout's record names, built more than half a day ago;
  - any other unit no build has read for two days, or `--older-than` days, never a day or less;
  - incremental state unchanged for three hours, or `--incremental-older-than` hours;
  - the target directory of a checkout that no longer exists;
  - the whole `target/` directory of a live checkout, a stray a build typed outside the Cargo verb
    left, where it stands apart from the shared directory and no Cargo process is working in it;
  - never a unit a live checkout's current outputs or record stand on, nor anything Cargo does not name
    by a unit hash, nor a `target/` a Cargo process is working in or that is the shared directory
    itself.
- **A lock a build holds is waited for** until that build lets go, however long it takes, the prune saying
  every thirty seconds which profile directory it waits on.
- **`--dry-run`** says what it would remove; **`--all`** clears the whole directory but Cargo's lock
  files, run by hand only.
- **Its ceiling is 120 GB, and 8 GB more for each live checkout that records its builds.** A timed prune
  that leaves it over sends a `cargo-target-over-ceiling` event, each run it stays over.
- **What it says:** how many units, incremental directories, loose files, checkout target directories and
  stray checkout `target/` directories it removed, about how much disk they held, the disk's free space
  before and after, and what the directory holds against its ceiling.
