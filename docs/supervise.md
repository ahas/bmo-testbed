# The supervisor: the set's services without systemd

## What it is

- **`bmo-testbed supervise serve` runs the set's services on a machine with no systemd user manager:** a
  container, a WSL distribution started without systemd, a packaged BMO on a person's own computer. It reads the
  same unit files a deployment links (`docs/deploy.md`, 「The unit links」), starts the enabled ones, watches
  each process, starts it again where its own `Restart=` says, stops it the way systemd would, and keeps each
  one's output in a log of its own.
- **It stands in for the user manager, not beside it.** Everything it starts finds stand-ins for `systemctl`,
  `systemd-run` and `journalctl` first on its PATH (「The stand-ins」), so the deployment, the install, the
  repositories' own deploy steps, after-steps and services call the user manager exactly as they do on a
  server, and their calls reach the supervisor. No consumer changes a line for it, and the testbed's deployment
  and install keep the one road they take under systemd.
- **Systemd stays the default.** Where a systemd user manager answers, the supervisor refuses to serve and the
  set runs under systemd; `--anyway` serves all the same, for a test on a machine that has one.
- **It knows no repository by name.** What it runs is what the unit directory holds; what a unit's health is,
  its repository's own declaration says (「Health」).

## The words

```
bmo-testbed supervise serve [--data-root <dir>] [--anyway]
bmo-testbed supervise status [--json] [--data-root <dir>]
bmo-testbed supervise env [--data-root <dir>]
bmo-testbed supervise stop-all [--data-root <dir>]
```

- **`serve`** runs the supervisor in the foreground, every enabled unit with it, until a SIGTERM, a SIGINT or
  `stop-all` stops every unit and ends it. It is started by whatever starts the person's session on the machine:
  the all-in-one desktop app, or a line of the machine's own start-up.
- **`status`** prints each unit's state, its main process, its restarts and its health, one line a unit;
  `--json` prints one object whose `units` holds each unit's state as the state file writes it.
- **`env`** prints the two lines that put a command under the supervisor: `PATH=` with the stand-ins' folder
  first, and `BMO_SUPERVISOR=` naming the control socket.
- **`stop-all`** stops every unit and ends the supervisor, as a SIGTERM to it does, and returns once it has.
- **`--data-root`** names the data root the supervisor keeps its folder under, over `BMO_DATA_DIR` and the
  default, `$HOME/.local/share/bmo/data`.
- **Exit codes:** 0 done; 1 it failed, the socket not reached among the faults; 2 refused: a systemd user
  manager answers, another supervisor holds the data root, or the words do not read.

## Where it stands

- **The units are read from the user unit directory,** `$XDG_CONFIG_HOME/systemd/user/` or
  `~/.config/systemd/user/`, the folder a deployment links each repository's units into. A link is followed to
  its file, so a flip of a deploy tree moves the units the supervisor reads with it, as it moves systemd's.
- **A unit is enabled as systemd keeps it:** a link in `default.target.wants/` for a service and in
  `timers.target.wants/` for a timer, under the unit directory, which the supervisor's `enable` writes and its
  `disable` removes. A unit directory enabled under the supervisor reads the same under systemd, and the other
  way round.
- **Its own folder is `supervise/` under the data root:**

| Path | What it is |
|---|---|
| `control.sock` | The control socket (「The control socket」) |
| `state.json` | Every unit's state and when each timer last fired, written whole after every change |
| `lock` | Held by the supervisor serving this data root, so a second one refuses |
| `logs/<unit>.log` | The unit's output, one line each with the time it was written, moved aside as `<unit>.log.1` past its cap |
| `bin/` | The stand-ins, `systemctl`, `systemd-run` and `journalctl`, each a link to the binary |

## What of a unit it reads

The unit language the set's own units are written in, read as systemd reads it: sections, repeated keys,
continued lines, quoting and the specifiers `%h`, `%u`, `%U`, `%t`, `%n`, `%N`, `%p`, `%i` and `%%`.

| Section | Key | How the supervisor takes it |
|---|---|---|
| `[Unit]` | `Description` | Shown in the unit's state |
| `[Unit]` | `After` | The order enabled units start in as the supervisor starts, among the units it starts then |
| `[Unit]` | `ConditionPathExists` | A start whose path does not stand, or stands where `!` says it must not, leaves the unit inactive and answers done |
| `[Unit]` | `StartLimitIntervalSec`, `StartLimitBurst` | More starts than the burst within the interval fail the unit as `start-limit-hit`, 10 seconds and 5 where none is named |
| `[Service]` | `Type` | `oneshot`: started once its process ended well, a start waiting for that; every other type is read as `simple`, started once its process runs |
| `[Service]` | `ExecStartPre`, `ExecStart` | Run in order, each line's words split as systemd splits them; the `-` prefix overlooks the line's failure, the other prefixes are read and need nothing of a user process |
| `[Service]` | `WorkingDirectory` | Where the processes start, the home where none is named; `-` overlooks a missing one |
| `[Service]` | `Environment`, `EnvironmentFile` | Laid over the supervisor's own environment in systemd's order, the files last and read as the unit starts; `-` overlooks a missing file |
| `[Service]` | `Restart`, `RestartSec` | Whether and after what pause an ended process starts again: `no`, `always`, `on-success`, `on-failure`, `on-abnormal`, `on-abort` and `on-watchdog`, as systemd weighs an ending |
| `[Service]` | `SuccessExitStatus` | The further codes and signals an ending counts clean by |
| `[Service]` | `RemainAfterExit` | A oneshot that ended well stays active |
| `[Service]` | `TimeoutStopSec` | How long a stop waits before it kills; `infinity` waits however long the process takes; 90 seconds where none is named |
| `[Service]` | `KillMode`, `KillSignal` | Who a stop signals and with what: the whole process group (`control-group`, the default), the main process alone (`process`), or the signal to the main process and the kill to the group (`mixed`) |
| `[Timer]` | `OnCalendar`, `Persistent`, `RandomizedDelaySec`, `OnBootSec`, `OnUnitActiveSec`, `Unit` | When the timer starts its service, in the machine's time zone; a persistent timer that missed an elapse while the supervisor was down fires as it starts |
| `[Install]` | `X-BMO-Deploy` | The deploy mark, kept in the unit's state; a `bot` unit's health is read (「Health」) |

- **Read and not applied:** `Slice`, `Nice`, `IOSchedulingClass`, `OOMPolicy`, `MemoryMax`, `MemoryHigh` and
  `MemorySwapMax` act through control groups and the scheduler a user process does not hold; a unit's state names
  each it carries under `notApplied`, so a reader knows which bounds do not hold here. Every other key is read
  past.
- **A unit file that does not read** stands with its load state `error` and the reason as its description, and
  a start of it fails saying so; the others run.

## How it runs a unit

- **Each process runs in a process group of its own,** which stands in for the control group systemd gives a
  unit: a stop signals the group, so what a service started goes with it, and once a main process ends, what it
  left in its group is sent the stop signal too, as systemd ends a unit's control group.
- **Its standard input is empty and its output is the unit's log,** standard output and standard error alike,
  line by line.
- **Its environment** is the supervisor's own, then each `Environment=`, then each environment file, with
  `BMO_SUPERVISOR` naming the socket and the stand-ins' folder first on whatever PATH the unit names, so a unit
  that sets its own PATH still finds them.
- **A process that ends** is weighed by its unit's `Restart=`: started again after `RestartSec`, the unit reading
  `activating (auto-restart)` while it waits and its restarts counted; left inactive where it ended clean; left
  failed otherwise, with how it ended as its result and its code or signal as its status.
- **A stop** signals as the unit's `KillMode` and `KillSignal` say, waits for the main process to end, and kills
  at the unit's bound where it has one. A unit stopped is never started again by its `Restart=`, and a stop
  during the pause before a restart cuts the pause short.
- **A restart** stops what runs, waits for its end, and starts the unit again; a restart of a unit that does not
  run starts it.
- **As it serves,** the supervisor reads every unit, starts the enabled services in the order their `After=` gives
  among them and arms the enabled timers. **As it ends,** it stops every unit in the reverse order, waits for each
  to end, writes its state file with no process named, and answers the stop.

## Transient runs

- **A `systemd-run` call is a transient service:** the command run as a unit of the name `--unit` gives, or a name
  of the supervisor's own, from the directory `--working-directory` names (the caller's with `--same-dir`, the home
  otherwise), with the supervisor's environment and each `--setenv` over it, never restarted. It outlives its
  caller, which is what a deployment run in a unit of its own needs (`docs/deploy.md`, 「Where a deployment
  runs」).
- **It is answered once its process runs,** saying the unit's name unless `--quiet`; with `--wait` once its
  process has ended, the stand-in ending with that process's code.
- **`--collect`** lets the unit go once it has ended, failed or not; without it a failed one stays to be read.
- **`--scope` and `--pipe`** run the command as the caller's own child, with each `--setenv` laid over the
  caller's environment, the stand-in ending with its code: a scope is the caller's process by definition, and a
  piped run hands its output back to the caller.
- **`--slice`, `--property` and the other resource words** are read and not applied, as a unit's are.

## Health

- **Each running unit marked `bot` has its repository's health asked** every 30 seconds: the deploy tree its unit
  file stands in is the nearest folder above the file its link names that holds a `testbed.toml`, and the health
  is that declaration's `[deploy] health` (`docs/declaration.md`), run from the tree with the supervisor's
  environment, as a deployment asks it. An answer of 0 within 30 seconds reads `healthy`, any other `unhealthy`; a unit whose
  repository declares none, or which does not run, reads `unknown`.
- **Health is read, never acted on.** A unit whose health fails is not restarted for it: a restart could cut
  work in hand, and the deployment, which waits on the health, is what rolls a release back. A screen, the
  all-in-one app's tray among them, shows it.

## The stand-ins

- **The binary called by a stand-in's name answers as that program.** `bmo-testbed` reads the name it was called
  by: called as `systemctl`, `systemd-run` or `journalctl` from the supervisor's `bin/`, it reads that program's
  words, asks the socket `BMO_SUPERVISOR` names (the data root's where none is named) and prints the answer in that
  program's own form. `--user` is read and needs nothing; there is no system manager to stand in for.
- **`systemctl`** answers the verbs the set calls:

| Verb | Answer |
|---|---|
| `start`, `stop`, `restart`, `reload-or-restart`, `try-restart` | Done once the unit has started, stopped or been replaced, as systemd's own call waits; a unit not found ends with 5 |
| `enable`, `disable` (with `--now`) | The wants link written or removed, a unit named by a path linked into the unit directory first, and started or stopped with `--now`; prints each link as systemd does |
| `daemon-reload` | The unit files read again; a running process keeps the file it started from until its next start |
| `reset-failed` | A failed unit read as inactive, its start limit counted from nothing |
| `kill` (`--signal`) | The signal sent as the unit's `KillMode` says |
| `is-active`, `is-enabled`, `is-failed` | The state a line each, nothing with `--quiet`, ending with 0 where any unit named is so and 3 (`is-active`) or 1 otherwise |
| `show` (`--property`, `--value`) | `Name=value` a line, a block a unit: `Id`, `Description`, `LoadState`, `ActiveState`, `SubState`, `Result`, `MainPID`, `NRestarts`, `ExecMainStatus` and `UnitFileState` |
| `status`, `list-units` | A short account of each unit; `status` ends with 3 where one is not active |
| `show-environment` | The environment every unit starts from |

  A verb it does not answer ends with 1, naming it.
- **`systemd-run`** answers as 「Transient runs」 says.
- **`journalctl`** reads the units' logs itself, with no socket: `--unit` once for each unit, `--lines` the last
  lines (all where none is named, ten with `--follow`), `--follow` reading on as the logs grow, and `--output cat`
  the lines alone; otherwise each line reads `<time> <unit>: <line>`, the units' lines merged in the order they
  were written. The other words a call passes are read past.

## The control socket

- **One JSON request a line, answered by one JSON line, and the connection closed.** A request names its `verb`,
  in kebab case, and what it acts on; an answer holds `code`, the code the matching systemd program would end
  with, `out` and `err`, what it would print, and `states`, the states a reading asked for. Its requests are the
  stand-ins' own: `start`, `stop`, `restart`, `try-restart`, `reset-failed`, `daemon-reload`, `enable`,
  `disable`, `kill`, `show`, `show-environment`, `run` and `shutdown`.
- **A request is answered when its work is done,** however long that takes: a stop once the process ended, a
  `run` with `wait` once its command ended. A caller that wants a bound sets its own.
- **The socket is the account's alone,** in a folder of the data root no other account reads.

## Running the install under it

- **A machine without systemd is installed the same way, under the supervisor.** Start `bmo-testbed supervise
  serve`, then run `bmo-testbed install` with the two lines `bmo-testbed supervise env` prints in its environment.
  Every command the install runs and every deployment it starts inherits them, so each `systemctl --user`, each
  `systemd-run --user` and each health the deployment reads reaches the supervisor; the testbed's own `needs:`
  takes the supervisor for the user manager and for lingering (`docs/install.md`, 「What a packager relies on」).
- **The supervisor must run for as long as the set does.** Nothing starts it at boot on such a machine but what
  the person's session runs; the all-in-one desktop app starts it as it starts, and stops it as it quits.
