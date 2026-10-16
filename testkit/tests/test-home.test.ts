import { existsSync, rmSync, statSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { expect, test } from "bun:test";

import { REFUSED_LOG, setUpTestHome, TEST_HOME_VARIABLE } from "../test-home";

// The home a test file runs in (docs/suite.md, 「The test home」), as `tests/preload.ts` asked for it,
// and the failure of a file that reached a stand-in, seen in a run of its own.

const home = process.env.HOME ?? "";
const fixtures = join(import.meta.dir, "fixtures");

test("the home is a directory of its own under the temporary directory, and is named", () => {
  expect(home.startsWith(join(tmpdir(), "bmo-test-home-"))).toBe(true);
  expect(process.env[TEST_HOME_VARIABLE]).toBe(home);
  expect(process.env.XDG_CONFIG_HOME).toBe(join(home, ".config"));
  expect(statSync(join(home, ".config")).isDirectory()).toBe(true);
});

test("a directory the ask names is made in the home", () => {
  expect(process.env.TESTKIT_PLACE).toBe(join(home, "place"));
  expect(statSync(join(home, "place")).isDirectory()).toBe(true);
});

test("the variables the ask names are emptied or removed, by name and by beginning", () => {
  expect(process.env.TESTKIT_EMPTIED).toBe("");
  expect("TESTKIT_REMOVED" in process.env).toBe(false);
  expect("TESTKIT_GONE_ONE" in process.env).toBe(false);
  expect("TESTKIT_GONE_TWO" in process.env).toBe(false);
});

test("no user service manager and no turn of the caller's is reachable", () => {
  expect(process.env.XDG_RUNTIME_DIR).toBe(join(home, "runtime"));
  expect(statSync(join(home, "runtime")).mode & 0o777).toBe(0o700);
  expect("DBUS_SESSION_BUS_ADDRESS" in process.env).toBe(false);
  expect(process.env.BMO_TESTBED_LOCK).toBe(join(home, "testbed.lock"));
  expect("BMO_TESTBED_HELD" in process.env).toBe(false);
});

test("the home is set up once", () => {
  expect(() => setUpTestHome({})).toThrow(/set up already/);
});

test("a stand-in comes first on the path, and starting it is written down and refused", () => {
  expect(Bun.which("testkit-stand-in")).toBe(join(home, "bin", "testkit-stand-in"));

  const ran = Bun.spawnSync(["testkit-stand-in", "an argument"]);
  const log = join(home, REFUSED_LOG);

  expect(ran.exitCode).toBe(97);
  expect(ran.stderr.toString()).toContain("a test started testkit-stand-in");
  expect(existsSync(log)).toBe(true);
  expect(Bun.file(log).size).toBeGreaterThan(0);

  // Taken away again, or this very file would fail as it ends.
  rmSync(log);
});

function runFixture(file: string): { exitCode: number; output: string } {
  const ran = Bun.spawnSync([process.execPath, "test", "--preload", "./preload.ts", `./${file}`], {
    cwd: fixtures,
    stdout: "pipe",
    stderr: "pipe",
  });

  return { exitCode: ran.exitCode, output: `${ran.stdout.toString()}${ran.stderr.toString()}` };
}

test("a file whose test started a stand-in fails as it ends, naming it, though its test passed", () => {
  const ran = runFixture("offender.fixture.ts");

  expect(ran.exitCode).not.toBe(0);
  expect(ran.output).toContain("this test file reached what tests must never reach");
  expect(ran.output).toContain("a test started the stand-in testkit-stand-in: an argument");
});

test("a file that reached no stand-in passes", () => {
  const ran = runFixture("clean.fixture.ts");

  expect(ran.output).toContain("1 pass");
  expect(ran.exitCode).toBe(0);
});
