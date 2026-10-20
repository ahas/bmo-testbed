---
name: ahas-testbed
description: Run heavy test and build commands through a machine's one queue with the bmo-testbed MCP server, and read what they came to. Use before running a whole test suite, a Cargo build or test, a package script that tests, checks or builds, or a browser test on a machine shared with other checkouts, and when reading a test run's counts and log, a change check, the lanes' queue, or a deployment's steps.
---

# ahas-testbed

A testbed holds one machine's heavy commands in line: two lanes, the wide one running one command on the
machine's whole width and the narrow one beside it on a quarter of it, so two suites never run each other
out of memory. Every heavy command started through it waits its turn, runs alone in its lane, and leaves
a test run in the testbed's record: its counts, its log and its state. The `bmo-testbed` MCP server runs
commands through that queue and reads the record back.

## Starting the server

Register the server with the harness as a stdio MCP server:

```
bmo-testbed mcp --service http://127.0.0.1:7460
```

The address is the testbed's HTTP service, which `bmo-testbed serve` starts; the reading tools call it,
behind the bearer token in the `BMO_TESTBED_TOKEN` environment variable of the server, or `--token`. The
running tools need no service: they start the server's own binary in the checkout named, so they work
wherever the binary does.

## What to do first

1. Run the heavy command with `testbed_run` instead of starting it in a shell:

   ```json
   { "command": ["cargo", "test", "-p", "my-crate"], "checkout": "/path/to/checkout" }
   ```

   Name `"queue": "narrow"` for a command that takes only a few threads, a front-end type check or build.
   The answer is a command run: `exitCode`, `logPath`, `waitedMs` (the wait for the turn), `spentMs` (the
   run itself) and `testRunId`.
2. Read what it came to with `testbed_test_run` on that `testRunId`: `state` (`passed`, `failed`,
   `not-run`, `running`, or `gone` where its process went without ending it), `passedCount`,
   `failedCount`, `skippedCount` and `logPath`. Read the log file for the failures; never run the command
   again only to see its output differently.

## Exit code 75

A command whose turn did not come within its wait ends with exit code 75 and runs nothing. Its place in
the line is kept for a few minutes: call the same tool with the same arguments again at once, and it waits
on from where it stood.

## A checkout's checks

- `testbed_check` runs the change check of a checkout: its quick checks, its declared static steps, and the
  tests its change touches, every step to its end, all in one log. `since` names the commit the change is
  read from; by default it is where the branch left the main branch.
- `testbed_suite` runs the checkout's whole suite, each unit in a process of its own. It can take many
  minutes; call it once, wait for its answer, and read its log.

Both answer a command run as `testbed_run` does; the suite's `testRunId` reads back with `testbed_test_run`.

## Seeing the machine

- `testbed_queues` shows each lane: its `width`, its `holder` and who is `waiting`, in the order they will
  be served, with `front` and `next` marking those who stand ahead of the ordinary line.
- `testbed_test_runs` pages the test runs, newest first; `checkout` narrows them to one checkout, and the
  `id` of the last one is the `after` of the next page while `more` is true.
- `testbed_deployments` and `testbed_deployment` read the deployments the testbed ran: their `state`, each
  step with its exit code and times, and `stoppedFor`, the stage a deployment stopped in where it did not go
  live.

## When a tool answers an error

A tool error's text is JSON: `status` where the service answered, `code` and `error`, the service's English
sentence. Match on `code`:

- `not-found`: nothing stands at the id asked for; the record keeps test runs and deployments a limited
  number of days.
- `bad-request`: the arguments are not the shape the tool takes; the sentence names what is wrong.
- `unauthorized`: the token is missing or wrong.

An error with no `code` means the service could not be reached, or the binary's verb could not be started;
check the address, that `bmo-testbed serve` runs there, and that the checkout exists.

The routes, members and tools behind each call are stated in the testbed's `docs/boundary.md` and in
`docs/openapi.json`, whose `x-tools` lists every tool's input and output.
