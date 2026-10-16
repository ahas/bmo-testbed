import { afterAll } from "bun:test";
import { chmodSync, existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs";
import { homedir, tmpdir } from "node:os";
import { join } from "node:path";

// The home a test file runs in, set up before the file's first line (docs/suite.md, 「The test
// home」). The consumer's own preload calls `setUpTestHome` once, handing in what differs per
// consumer: the variables to take away, the programs to stand in for and the directories of the home
// to name. What follows is what every consumer's home has.

// The variable naming the home, so a process that carries it is a test.
export const TEST_HOME_VARIABLE = "BMO_TEST_HOME";

// The file in the home a stand-in writes to when a test starts it.
export const REFUSED_LOG = "refused";

// What a consumer hands in. Every list is the consumer's; the test kit names none.
export interface TestHomeAsk {
  // Variables set empty: for a reader that takes empty as unset, where an env file would otherwise
  // hand the value back.
  emptied?: readonly string[];
  // Variables removed by name: for a reader that takes a present empty value as given.
  removed?: readonly string[];
  // Variables removed by the beginning of their name.
  removedPrefixes?: readonly string[];
  // Programs that must never be reached; each gets a stand-in at the front of `PATH`.
  standIns?: readonly string[];
  // Variables naming a directory of the home, made before the first test: the variable, and the path
  // within the home.
  directories?: Readonly<Record<string, string>>;
}

// What the home is once set up.
export interface TestHome {
  home: string;
  bin: string;
}

const homePrefix = "bmo-test-home-";

// How old a home has to be before another file takes it away: a file killed before its end leaves its
// home behind, and an hour on it belongs to no run still going.
const staleMs = 60 * 60_000;

// The exit status a stand-in ends with.
const refusedStatus = 97;

let setUp: TestHome | null = null;

export function setUpTestHome(ask: TestHomeAsk = {}): TestHome {
  if (setUp !== null) {
    throw new Error(`the test home is set up already, at ${setUp.home}; a consumer's preload sets it up once`);
  }

  sweepStale();

  const realHome = homedir();
  const home = mkdtempSync(join(tmpdir(), homePrefix));
  const bin = join(home, "bin");

  mkdirSync(bin, { recursive: true });
  mkdirSync(join(home, ".config"), { recursive: true });

  for (const name of ask.standIns ?? []) {
    writeStandIn(home, bin, name);
  }

  keepFromRealHome(realHome);

  process.env.HOME = home;
  process.env.XDG_CONFIG_HOME = join(home, ".config");
  process.env[TEST_HOME_VARIABLE] = home;
  process.env.PATH = `${bin}:${process.env.PATH ?? ""}`;

  for (const [variable, path] of Object.entries(ask.directories ?? {})) {
    const directory = join(home, path);

    mkdirSync(directory, { recursive: true });
    process.env[variable] = directory;
  }

  for (const name of ask.emptied ?? []) {
    process.env[name] = "";
  }

  const prefixes = ask.removedPrefixes ?? [];

  for (const name of Object.keys(process.env)) {
    if (prefixes.some((prefix) => name.startsWith(prefix))) {
      delete process.env[name];
    }
  }

  for (const name of ask.removed ?? []) {
    delete process.env[name];
  }

  // No user service manager: a runtime directory holding no socket, and no bus address, so a
  // `systemctl --user` or `systemd-run --user` under test reaches nothing whether or not the caller's
  // line could.
  const runtimeDir = join(home, "runtime");

  mkdirSync(runtimeDir, { recursive: true, mode: 0o700 });
  process.env.XDG_RUNTIME_DIR = runtimeDir;
  delete process.env.DBUS_SESSION_BUS_ADDRESS;

  // The machine's turn for heavy commands is held by the run this file stands in, so a heavy command a
  // test starts takes its turn in this home instead and never waits on its own parent, and the
  // holder's mark the run handed down names nothing here.
  process.env.BMO_TESTBED_LOCK = join(home, "testbed.lock");
  delete process.env.BMO_TESTBED_HELD;

  launchInCurrentEnvironment();

  afterAll(() => {
    const log = join(home, REFUSED_LOG);
    const refused = existsSync(log) ? readFileSync(log, "utf8").trim() : "";

    rmSync(home, { recursive: true, force: true });

    if (refused !== "") {
      throw new Error(`this test file reached what tests must never reach:\n${refused}`);
    }
  });

  setUp = { home, bin };

  return setUp;
}

function sweepStale(): void {
  const cutoff = Date.now() - staleMs;

  for (const name of readdirSync(tmpdir())) {
    if (name.startsWith(homePrefix)) {
      try {
        if (statSync(join(tmpdir(), name)).mtimeMs < cutoff) {
          rmSync(join(tmpdir(), name), { recursive: true, force: true });
        }
      } catch {
        // Taken away by another file's sweep in the same moment.
      }
    }
  }
}

// A stand-in under a name that must never reach the real program. It writes down that it was called
// and with what, and ends as a failure; the file it ran in fails when it ends.
function writeStandIn(home: string, bin: string, name: string): void {
  const path = join(bin, name);

  writeFileSync(
    path,
    [
      "#!/bin/sh",
      `printf '%s\\n' "a test started the stand-in ${name}: $*" >> "${join(home, REFUSED_LOG)}"`,
      `echo "a test started ${name}; tests reach only fakes of their own" >&2`,
      `exit ${refusedStatus}`,
      "",
    ].join("\n"),
  );
  chmodSync(path, 0o755);
}

// What the tests still read from the real home, named where it is so that moving the home does not
// move it, unless the caller named it already: git's global configuration, so a test that commits in
// a temporary repository keeps its author; the user cache; and the browser builds.
function keepFromRealHome(realHome: string): void {
  if (process.env.XDG_CACHE_HOME === undefined) {
    process.env.XDG_CACHE_HOME = join(realHome, ".cache");
  }

  const gitConfig = join(realHome, ".gitconfig");

  if (process.env.GIT_CONFIG_GLOBAL === undefined && existsSync(gitConfig)) {
    process.env.GIT_CONFIG_GLOBAL = gitConfig;
  }

  const browsers = join(realHome, ".cache", "ms-playwright");

  if (process.env.PLAYWRIGHT_BROWSERS_PATH === undefined && existsSync(browsers)) {
    process.env.PLAYWRIGHT_BROWSERS_PATH = browsers;
  }
}

// The runtime starts a program on the environment the process began with unless it is handed one, and
// looks a program up on the PATH it began with: the moves above would reach nobody a test or the code
// under it starts without naming an environment. So within a test file, starting a program and
// looking one up both default to the environment as it stands now.
function launchInCurrentEnvironment(): void {
  type Spawn = typeof Bun.spawn;
  type SpawnSync = typeof Bun.spawnSync;
  type Which = typeof Bun.which;

  const spawn = Bun.spawn;
  const spawnSync = Bun.spawnSync;
  const which = Bun.which;

  function withEnv<T>(first: unknown, options: unknown, run: (first: unknown, options: unknown) => T): T {
    if (Array.isArray(first)) {
      const given = (options ?? {}) as { env?: unknown };

      return run(first, { ...given, env: given.env ?? process.env });
    }

    const given = first as { env?: unknown };

    return run({ ...given, env: given.env ?? process.env }, undefined);
  }

  const runtime = Bun as unknown as Record<"spawn" | "spawnSync" | "which", unknown>;

  runtime.spawn = ((first: unknown, options?: unknown) =>
    withEnv(first, options, (cmd, opts) => (spawn as unknown as (a: unknown, b: unknown) => unknown)(cmd, opts))) as unknown as Spawn;
  runtime.spawnSync = ((first: unknown, options?: unknown) =>
    withEnv(first, options, (cmd, opts) => (spawnSync as unknown as (a: unknown, b: unknown) => unknown)(cmd, opts))) as unknown as SpawnSync;
  runtime.which = ((command: string, options?: { PATH?: string; cwd?: string }) =>
    which(command, { ...options, PATH: options?.PATH ?? process.env.PATH ?? "" })) as Which;
}
