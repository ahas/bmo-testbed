import { plugin } from "bun";

// What a test run must not let any library reach for: the process's own standard error
// (docs/suite.md, 「The test home」).
//
// Reaching for it builds the runtime's own stream on that descriptor, and a build made while a
// worker still holds an earlier one is refused with EEXIST from epoll_ctl. The refusal arrives as
// an error nobody listens for, which the import that caused it cannot catch, so the file whose
// import reached for it drops out before its first test with its tests uncounted altogether.
//
// This module is preloaded ahead of the home, because the home reaches `debug` through its own
// imports and an import is evaluated before the first line of the file that asked for it.

// `debug` asks standard error whether it is a terminal, and only where nothing has answered.
// Answered here, the question is never put to the descriptor, and a caller that answered it already
// keeps its own answer.
process.env.DEBUG_COLORS ??= "false";

// A module that takes standard error outright as it loads cannot be answered, only kept from loading:
// the consumer names it and what it exports, and this stands those exports in under its name. A
// consumer's preload calls it before any test imports the module.
export function standInModule(name: string, exports: Record<string, unknown>): void {
  plugin({
    name: `a stand-in for ${name}`,
    setup(build) {
      build.module(name, () => ({ loader: "object", exports }));
    },
  });
}

// A class of a stand-in module that refuses to be built, telling whoever asked what they reached for:
// a test reaches only fakes of its own.
export function refused(module: string, what: string): new (...args: unknown[]) => never {
  return class {
    constructor() {
      throw new Error(`a test asked the stand-in for ${module} for ${what}; a test reaches only fakes of its own`);
    }
  } as unknown as new (...args: unknown[]) => never;
}
