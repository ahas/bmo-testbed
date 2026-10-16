import { expect, test } from "bun:test";

// A file whose one test passes but starts a stand-in on the way, which its run must fail for.
test("starts a stand-in and passes", () => {
  const ran = Bun.spawnSync(["testkit-stand-in", "an argument"]);

  expect(ran.exitCode).toBe(97);
});
