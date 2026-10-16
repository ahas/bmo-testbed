import { expect, test } from "bun:test";

import { STOOD_IN_MODULE } from "./preload";

// The preload that keeps every library of a run away from the process's own standard error
// (docs/suite.md, 「The test home」): the question answered for the one that asks, and a stand-in for
// a module the consumer names.

test("the colours question is answered, so the library that asks never puts it to the descriptor", () => {
  expect(process.env.DEBUG_COLORS).toBe("false");
});

test("a module the consumer names is a stand-in, and asking it for anything says so", async () => {
  const stood = (await import(STOOD_IN_MODULE)) as unknown as {
    version: string;
    Client: new (...args: unknown[]) => unknown;
  };

  expect(stood.version).toBe("0.0.0-test-stand-in");
  expect(() => new stood.Client("wss://example.invalid")).toThrow(/reaches only fakes of its own/);
});
