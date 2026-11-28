# Installing a set: `install`

## What it is

- **The `install` verb lays a set of repositories onto a fresh machine and keeps it whole:** it clones each one,
  checks what the machine must already hold, makes the tokens and the databases, writes the settings, asks for
  what only a person has, puts each repository's first release live through the deployment
  (`docs/deploy.md`), runs what each declares for after its first release, and reads every service's health.
  `bmo install` reaches it (「The nouns `bmo` serves」 in `docs/boundary.md`).
- **It is written for an agent to follow, never for a person to answer.** It asks nothing on a terminal: every
  step prints one JSON line, and a value it lacks is a step that says which value, how to give it, and what to
  run next (「What it prints」).
- **It knows no repository by name.** Which repositories a kind of install holds stands in a presets file of the
  index the set is listed in (「The presets file」); what one repository needs to be installed stands in that
  repository's own `install.toml` (「The install declaration」). A repository added to a preset is a change to
  the presets file alone.
- **Running it again continues it.** Every step first reads whether it is already done, from the machine itself
  and never from a record of having done it: a checkout that stands is not cloned again, a token or a settings
  file that stands is never rewritten, a database that stands is not made, a repository whose deploy tree stands
  is not deployed, and an after-step whose own check answers done is not run. A run that stopped anywhere is
  taken up again by the same command.
- **Its sibling verb, `upgrade`, moves an install to a newer state of the set** (「Upgrading: `upgrade`」).

## The words

```
bmo-testbed install --preset <name> [--with <repository>]… [<places>] [--dry-run]
bmo-testbed install --add <repository> [<places>] [--dry-run]
bmo-testbed install --status [<places>]
bmo-testbed install --set <input> [<places>]

<places>: [--index <dir>] [--data-root <dir>] [--prefix <dir>] [--postgres-host <host>] [--postgres-port <port>]
          [--valkey <address>]
```

- **`--preset`** installs the named preset whole. `--with` names the repository chosen for each of the
  preset's choices (「The presets file」); a choice left unnamed is a `needs-input` step naming its repositories.
- **`--add`** installs one more repository into the install the data root holds, with the console features it
  turns on (「The console features」). A repository already installed is a step that says so. On a machine already
  serving people it puts a new service live beside the running ones, so it is the supervising agent's alone, run
  only on the owner's approval for that one repository; an agent working on a job never runs it, and asks its
  supervisor instead.
- **`--status`** changes nothing: it prints, for every repository of the install the data root holds, whether its
  checkout stands and the commit its deployment record names, `ok` where it stands and is live or deploys nothing,
  `failed` otherwise, and then every port the install chose, one `port:<name>` line each.
- **`--set`** stores one input's value, read whole from standard input with one last line ending taken off, where
  the declaration of the input says (「Inputs」), found among the declarations of every repository the index holds a
  checkout of, and prints one line naming where it went. The value is never an argument, so it stands in no
  process listing and no shell history.
- **`--index`** names the index checkout. Where it is not given, `BMO_INDEX` names it, else the git top level of
  the directory the verb runs in, where that holds the presets file. An index that cannot be found is a refusal.
- **`--data-root`** names the data root the install writes its settings, tokens and inputs into and keeps its
  own state in, over `BMO_DATA_DIR` and the default, `$HOME/.local/share/bmo/data`. Where each repository's
  services read their settings is its own declaration's (`docs/declaration.md`), so a data root other than the one
  the declarations name suits a dry run, or a machine whose declarations name it, and nothing else.
- **`--prefix`** names the install prefix, `.local/share` under the home directory where none is named: every
  repository's deploy root and data root must stand under it or under the data root (「The order of a run」, step 5).
- **`--postgres-host` and `--postgres-port`** name the PostgreSQL server the databases are made on and the services
  reach, over `PGHOST` and `PGPORT`, the system's socket folder `/var/run/postgresql` and `5432` where neither names
  one; every check, build, after-step, restart and verification the install runs is handed the same `PGHOST` and
  `PGPORT`, so a step's own database client reaches that server alone; **`--valkey`** names the Valkey the console keeps its sessions in, over `BMO_INSTALL_VALKEY`,
  `redis://127.0.0.1:6379` where neither does. A packager that bundles its own servers names them here.
- **`--dry-run`** reads everything and changes nothing: every step that would act prints `planned`. It runs no
  after-step's check, no health and no verification, since those reach the services an install sets up.

## What it prints

- **One JSON object per line on standard output, one per step,** with four members:

  | Member | Meaning |
  |---|---|
  | `step` | The step's name: its kind, then a colon and what it acts on where it acts on one thing (`clone:bmo-web`, `token:web.server`) |
  | `status` | `ok` (done, now or before), `planned` (a dry run's step that would act, or a settings file or a line put off until a file it names stands, written later in the run), `needs-input` (a value is missing), `failed` (it could not be done) |
  | `detail` | One English sentence: what was done, what is missing, or what went wrong |
  | `next` | What the agent does next, as a command where one answers it, or an empty string where nothing is owed |

- **A `needs-input` line's `next` is the command that gives the value:** `bmo install --set <input>` with the
  value on standard input, and its `detail` says what the value is and where a person finds it.
- **The last line is the step `done`,** whose `detail` says how the install stands and whose `next` names the
  console's address where the install brings one (「The report」).
- **Exit codes:** 0 where every step is `ok` or `planned`; 3 where none failed and some need input; 1 where a step
  failed; 2 for a refusal before any step: words that do not read, no index, a presets file or a declaration that
  does not read, an unknown preset or repository.
- **Every step runs to its end.** A deployment is waited for until its unit ends (「Deploying」), and nothing is cut
  short by a clock.

## The order of a run

1. **`preset`:** the presets file read, the preset resolved through what it extends, its choices settled by `--with`,
   and the repositories to clone and to install listed in their order. A preset's choice left open stops the run
   here with `needs-input`.
2. **`clone:<repository>`:** each repository of the preset's `clone` and `units` cloned into the index checkout's
   folder the presets file names for it, from the address it names. A checkout that stands is fetched, but in a
   dry run, and left on whatever branch it stands on. Then **`checkouts`:** every checkout standing named in the
   machine's registry of checkouts (`docs/suite.md`, 「The dependents of a major change」) where it is not yet, its
   other lines kept, since the set's later deployments read it: the hosted deploy tool finds a repository's
   checkout there, and a deployment's gate the repository holding the shared versions.
3. **`declaration:<repository>`:** each installed repository's `install.toml` read, and its `testbed.toml` with
   it. A repository with no install declaration fails here: its settings are its own documents' to give, and the
   install does not guess them.
4. **`needs:<program>`:** every program a declaration needs, once each, asked whether it stands. One missing whose
   need names its source is built from it (「A program built from its source」). One missing otherwise, or whose
   build did not make it stand, is `failed`, its `detail` saying what it is for and its `next` the declaration's
   own words on where it comes from. The run stops after this stage where any program is missing.
5. **`place:<repository>` and `port:<name>`:** every repository's deploy root and data root read against the
   install prefix and the data root, one standing elsewhere failed; then every port chosen (「Ports」).
6. **`input:<name>`:** every input the installed repositories declare, read where it is kept. A required input not
   given is `needs-input`; an optional one not given stands empty. The run stops after this stage where any is
   missing, before anything is written, so the agent can gather every value at once.
7. **`token:<name>`:** each declared token made, random, where its file does not stand.
8. **`database:<name>`:** each declared PostgreSQL database made where it does not stand.
9. **`settings:<file>` and `line:<file>:<key>`:** each declared settings file written whole where it does not
   stand, and each declared line added to its file where the file holds no line of that key, in the order the
   repositories stand. A settings file or a line one of whose values is a file another repository makes once it
   runs is put off until that file stands (「Values」).
10. **`console-features`:** the console features file written from the preset and the repositories installed
    (「The console features」).
11. **For each repository in order, `deploy:<repository>`** and then each of its **`after:<repository>:<id>`**:
    the first release put live (「Deploying」), then its after-steps (「After-steps」). Before each deployment the
    settings put off are written where their files now stand, and the repository whose settings changed after it
    went live is restarted by its declared `restart` command.
12. **`verify:<repository>`:** each installed repository's declared health asked once, and its declared
    verifications run (「Verifications」).
13. **`done`.**

## Deploying

- **A repository whose declaration deploys nothing,** a content set or a base library, has no `deploy` step; its
  after-steps run in its place.
- **A repository is deployed by the install only where its deploy tree does not stand yet.** One deployed already
  is `ok` with the commit its record names; its later releases go by the deployment road (`docs/deploy.md`).
- **The commit is the head of the main branch the declaration names,** fetched from the remote, and it is deployed
  with `--no-check`, since every commit on a main branch passed its close's suite and a fresh machine holds no record
  of it to take instead, and with `--force-live`, since nothing of the repository is live for a quiet to count, and
  a quiet that reads another repository not deployed yet could never tell.
- **The repository's declared data root is made before its first deployment** where it does not stand, since the
  deployment refuses a data root that does not exist (`docs/deploy.md`) and on a fresh machine nothing has made it.
- **A first deployment is not a harness's deployment.** `deploy` refuses a process a harness started, so its
  session never cuts a live service short (`docs/deploy.md`, 「What it refuses」). A repository with no deploy tree
  has nothing live to cut, so the install hands its first deployment on without the harness's mark; the install
  never deploys a repository whose deploy tree stands.
- **The deployment is handed the caller's `PATH` with `.local/bin` under the home directory before it,** where `bmo`
  and every unit's own command are linked, since a later repository's deploy steps call them and a fresh machine's
  shell may not hold that folder yet.
- **The deployment runs in its own unit,** as every deployment does, and the install waits for that unit to end,
  then reads the deployment record: `ok` where it names the commit, `failed` otherwise, its `next` the command that
  reads the unit's journal.
- **The repositories deploy one at a time, in the order the preset lists them,** so one whose deploy steps call
  another's command finds it live.

## Upgrading: `upgrade`

```
bmo-testbed upgrade [--nightly] [--index <dir>] [--data-root <dir>] [--prefix <dir>] [--dry-run]
```

- **`upgrade` moves the install the data root holds to a newer state of the set,** every repository at once: it
  brings each checkout of the install to the one target, then puts each installed repository's commit live through
  the deployment, one after the other in the install's order. `bmo upgrade` reaches it. It installs nothing new: a
  repository the install does not hold stays out, and one more is `install --add`'s.
- **The target is the newest release, or the nightly.** Without `--nightly` every checkout goes to the newest
  release tag it holds (「The deployed version」 in `docs/deploy.md` names which tags are releases), a stable
  release, an alpha, a beta or a release candidate, checked out as that commit; a checkout holding no release tag
  is a `failed` step. With `--nightly` every checkout goes to the head of its main branch, the one its testbed
  declaration names, `main` where it has none, the branch checked out and moved forward to its remote's head; the
  deployment names it the day's nightly.
- **A checkout is only ever moved forward, and never over work.** Each checkout is fetched first. One holding
  changes not committed, standing on a branch other than its main branch, or whose main branch cannot move to its
  remote's head without a merge, is a `failed` step that names which, and the run stops after the checkouts, since a
  service builds against the base libraries checked out beside it and one left behind would build a mixed set.
- **Each repository is deployed as a later release is,** waiting for quiet: without `--no-check`'s gate, since every
  commit on a main branch and every release tag passed its suite, and without `--force-live`, so a service at work
  is not cut short; a repository whose live commit is the target already is `ok` and is not deployed again, and one
  that deploys nothing is `ok`. A deployment is waited for until its unit ends, and one that does not go live stops
  the run there, its line naming the deployment's journal, so no later repository goes live on a set that broke.
- **It prints as `install` does** (「What it prints」): a `target` line naming what it moves to, a
  `checkout:<repository>` line for every checkout of the install, cloned and installed alike, a `deploy:<repository>`
  line for every installed repository, and the step `done`. `--dry-run` fetches nothing and moves nothing: it reads
  every checkout as it stands and prints each move and each deployment as `planned`.
- **It runs outside the services it moves.** It restarts every service of the install, the agents' among them, so
  an agent working inside the install never runs it; a person runs it, or an agent in a terminal of its own. Asked
  so, it hands its deployments on without a harness's mark, as an install hands its first ones (「Deploying」): the
  restart is what was asked for.

## The presets file

- **`presets.toml` at the index checkout's root,** read by a TOML library against this schema, a key it does not
  name refused.

| Key | Type | Meaning |
|---|---|---|
| `presets` | integer | The major of this schema the file is written for: `1` |
| `[[repository]]` | table | One repository of the set: `name`, its declared name; `folder`, where it stands in the index checkout; `url`, the address it is cloned from; `features`, the console features installing it turns on (「The console features」) |
| `[preset.<name>]` | table | One preset: `summary`, one English sentence; `extends`, the preset whose lists stand before its own; `clone`, the repositories checked out but not installed, which others build against; `units`, the repositories installed, in the order they are deployed; `[[preset.<name>.choose]]`, a choice of exactly one of `units` among several, with `what`, what is chosen, in English; `features`, the console features the preset turns on |

- **A repository named in a preset and not among the `[[repository]]` tables** refuses the file, and so does a
  preset extending one that does not stand, or extending in a ring.

## The install declaration

`install.toml` at a repository's root, read from the checkout as it stands. A key the schema does not name is
refused, naming it.

| Key | Type | Meaning |
|---|---|---|
| `install` | integer | The major of this schema the declaration is written for: `1` |
| `restart` | command | Restarts the repository's services, run where a settings file it reads changed after its first release |
| `[[needs]]` | table | A program the machine must hold: `program`, asked with `command -v`, or `check`, a command answering 0 where it stands; `what`, what it is for and where it comes from, in English; `source`, where the install builds it from where it is missing (「A program built from its source」) |
| `[[port]]` | table | A port the repository listens on: `name`, `number` |
| `[[token]]` | table | A random token: `name`; `file`, where it is kept under the data root; `bytes`, its length before encoding, 32 by default; `encoding`, `hex` or `base64`, `hex` by default |
| `[[input]]` | table | A value only a person has (「Inputs」): `name`, `what`, `file`, `secret`, `optional`, `default` |
| `[[value]]` | table | A value of the install's own: `name`, `value`, a template |
| `[[database]]` | table | A PostgreSQL database the settings name: `name` |
| `[[settings]]` | table | A file written whole where it does not stand: `file`, a path read as every path here is (below); `text`, a template; `mode`, `600` by default |
| `[[line]]` | table | A `KEY=VALUE` line added at the end of a file another declaration writes, where the file holds no line beginning with the key and `=`: `file`, `key`, `value`, a template. A line waits for its file, so one added to a file put off is put off with it |
| `[[after]]` | table | A step run once the repository's first release is live (「After-steps」): `id`, `run`, `done`, `inputs` |
| `[[verify]]` | table | A check run once everything is installed (「Verifications」): `id`, `run`, `what` |
| `report` | template | The address the install reports at its end, where the repository is the console (「The report」) |

- **A path a declaration names** is read under the data root, unless it begins with `~/`, which stands for the
  home directory, or with `prefix:`, which stands for the install prefix.

### Ports

- **A declared port is the number preferred, not a number held.** Where nothing listens on it, or its repository is
  live already, it is the port; where another program holds it, the install takes the next free number above it,
  one no other port of the install has, and says so on the port's line.
- **The ports chosen are kept** in `install/ports.json` under the data root, by name, and every later run reads
  them first, so a port never moves once chosen and a packager or a screen reads every service's port from the one
  file. Every template's `{{port.<name>}}` is the number kept.

### A program built from its source

- **A need may name its source,** a table of four keys: `name`, the folder its repository is cloned into under the
  data root's `install/sources/`; `repository`, the address it is cloned from; `rev`, the tag or commit checked
  out; and `build`, a command run from the clone that builds the program and places it where the need asks for it.
  A program no system package carries, such as a terminal host published as source alone, is installed so.
- **Only a missing program is built.** Where the need stands nothing is cloned. Where it does not, the repository is
  cloned, or fetched where its clone stands, `rev` is checked out, `build` runs, and the need is asked again: `ok`
  where it stands now, `failed` otherwise with what the build said, its `next` to mend that and run the install
  again. A dry run plans the build and runs nothing.
- **The build is a command like any other:** run with the environment every command is handed (「After-steps」),
  never a template. What the build itself needs, a compiler of a given version among it, is its `what`'s to name,
  and a build that lacks it fails saying so.

### Values

A template is text in which `{{` a value's name `}}` stands for that value, filled when the step that writes it
runs. A name nothing declares fails the step naming it.

| Mark | Stands for |
|---|---|
| `{{token.<name>}}` | The token's value |
| `{{port.<name>}}` | The port's number |
| `{{input.<name>}}` | The input's value, or its default, or empty where it is optional and not given |
| `{{value.<name>}}` | A declared value, its own template filled |
| `{{file.<path>}}` | The content of the file at the path, its last line ending taken off; a file another repository makes once it runs, so what names it is put off until it stands |
| `{{data_root}}`, `{{home}}`, `{{index}}`, `{{prefix}}` | The data root, the home directory, the index checkout and the install prefix |
| `{{postgres.host}}`, `{{postgres.port}}`, `{{valkey.url}}` | The PostgreSQL server and the Valkey the words name, which every settings file names its database and sessions by |
| `{{checkout.<repository>}}` | That repository's checkout |
| `{{user}}` | The account the install runs as |

- **Names are shared by the whole install,** so one declaration names what another declares: a consumer's settings
  name its upstream's port and token. A name declared twice refuses the install, naming both declarations.
- **A filled value is never printed:** a line names a token, an input or a file by its name, never its content.

### Inputs

- **An input is kept where its declaration says,** `file` under the data root: a secret input, `secret = true`,
  as a file of mode 600 in the secret store, the data root's `env/`, which the console's secret store lists;
  any other in the same way, readable by its owner alone. `--set` writes it whole and renames it over its name.
- **A required input that stands nowhere is `needs-input`,** its `detail` the declaration's `what`, which says what
  the value is and where a person finds it, and its `next` the `--set` command.
- **An input with a `default`** is filled with the default where none is given, and is never asked. The default is
  a template, so it may name a port the install chose.

### After-steps

- **`run` and `done` are commands, not templates,** run from the checkout once the repository's first release is
  live; what they need of the install they read from the settings the install wrote. A `[[verify]]`'s `run` is the
  same.
- **The command's environment** holds `BMO_DATA_DIR` naming the data root, `BMO_INSTALL_INDEX` the index
  checkout, and for each input the step names in `inputs` a variable `BMO_INPUT_<NAME>` naming the file the input
  is kept in, its name in capitals with every character that is not a letter or a digit written `_`. A value
  never stands on its command line. Every command, a need's check and a source's build, a restart's and a
  verification's among them, is handed `BMO_DATA_DIR` and `BMO_INSTALL_PREFIX`, the install prefix, and `USER` and
  `LOGNAME` naming the account the install runs as: `USER` where the install's environment names one, else the
  password database's name for its user id, since a container's command carries none and a database client takes
  it for its role.
- **`done` is a command answering 0 where the step's work already stands,** asked first, so a rerun skips it.
  A step whose `run` ends with any other code than 0 is `failed`, and the run stops there.

### Verifications

- **Each installed repository's health is asked once,** the `health` its deployed declaration names
  (`docs/declaration.md`), and a repository that names none is read by its `bot` unit's state alone.
- **Each `[[verify]]` runs once everything is installed:** a test message sent and read back, a page answered.
  A failure is `failed` with the declaration's `what`.

### The report

- **The console's address is the `report` template of the repository that declares one,** filled; the `done`
  line's `next` names it, so the agent tells the person where to sign in.

## The console features

- **A feature is a part of the console's own that an install may leave out,** such as its organisations, which a
  person alone never needs. Which ones the console reads, and what each leaves out, is the console's own
  documents' (bmo-web's `docs/web.md`, 「The console's features」).
- **The features file is written whole by every run that changes the install:** each feature the preset or an
  installed repository turns on is `true`, and each feature any preset or repository of the presets file names is
  `false` otherwise. It stands at the data root's `env/console-features.toml`.

## What a packager relies on

The install is also the engine of a packaged BMO: one installer on a person's own computer that lays the personal
preset down and starts and stops it itself. These hold for every declaration and every run, so such a packager
runs this verb unchanged:

- **One data root and one prefix, both given.** Everything an install writes stands under the data root or the
  prefix the words name; a declaration names its paths through `{{data_root}}`, `{{prefix}}` and `prefix:` and never
  as a fixed folder of one machine, and a repository whose declared deploy root or data root stands outside both is
  a failed `place:` step, never a silent split.
- **No server assumed system-wide.** PostgreSQL and Valkey are whatever the words name; a declaration reaches them
  through `{{postgres.host}}`, `{{postgres.port}}` and `{{valkey.url}}`, so bundled servers serve as well as the
  system's.
- **Ports chosen and kept, never fixed.** A port that is taken moves, and every number stands in `install/ports.json`
  (「Ports」).
- **Progress read as data.** Every step is one JSON line and `--status` answers each repository as one, so a progress
  screen reads them as they come and never parses a sentence; `detail` is for a person, `step` and `status` are for
  the screen.
- **Each repository's services as processes.** A repository's services are user units of systemd on a server, which
  its deployment keeps (`docs/deploy.md`, 「The unit links」); each unit runs one program with its settings read from
  the data root, so the same unit files run under a supervisor that is not systemd. Systemd stays the default on a
  server.
- **A machine without systemd runs the set under the testbed's supervisor** (`docs/supervise.md`). The packager
  starts `bmo-testbed supervise serve` and runs this verb with the lines `bmo-testbed supervise env` prints in its
  environment: the stand-ins first on the PATH, `BMO_SUPERVISOR` naming the supervisor's socket and the runtime
  directory every unit reads. Every deployment,
  restart, after-step and health read then reaches the supervisor through the same `systemctl --user`,
  `systemd-run --user` and `journalctl --user` calls it makes under systemd, and the testbed's own `needs:` takes the
  supervisor for the user manager and for lingering, since whatever starts the supervisor starts the set. The
  all-in-one desktop app is such a packager.

## The install's own state

- **`install/preset.json` under the data root** names the preset installed, the repositories chosen and those
  added, written once the `preset` step resolves. `--add` and `--status` read it; a data root that holds none has no install to add to.
- **`install/ports.json`** keeps the ports chosen (「Ports」).
- **Nothing else is kept:** whether a step is done is read from the machine each time.
