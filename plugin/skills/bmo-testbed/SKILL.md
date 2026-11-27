---
name: bmo-testbed
description: How the bmo-testbed MCP tools run heavy test and build commands through the machine's one queue and read what they came to: a test run's counts and log, a change check, the lanes' queue and a deployment's steps.
---

# bmo-testbed

A testbed holds one machine's heavy commands in line: two lanes, the wide one of a few seats sharing the
machine's width and the narrow one beside it, one seat on a quarter of it, a further seat taken only while
the machine has memory to spare. Every heavy command started through it waits its turn, runs in its seat
on its share of the threads, a deployment holding every seat, and leaves a test run in the testbed's
record: its counts, its log and its state. The `bmo-testbed` MCP server runs
commands through that queue and reads the record back.

## Starting the server

Register the server with the harness as a stdio MCP server:

```
bmo-testbed mcp --service <the testbed service's address>
```

The address is the testbed's HTTP service, which `bmo-testbed serve` starts at the loopback address its policy names, or `BMO_TESTBED_SERVICE` gives; never guess one; the reading tools call it,
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

A command waits for its turn however long the line takes, and runs until it ends. One ends with exit code
75 and runs nothing only where the harness it runs for went away before its turn came; call the same tool
with the same arguments again.

## A checkout's checks

- `testbed_check` runs the change check of a checkout: its quick checks, its declared static steps, and the
  tests its change touches, every step to its end, all in one log. `since` names the commit the change is
  read from; by default it is where the branch left the main branch. A change that spans several
  repositories is checked once with `checkouts`, each by its `path` and its own `since`: every checkout is
  planned by its own declaration and the set runs as one run on the placed machine, every line naming its
  repository and the last lines naming each failed repository and step.
- `testbed_suite` runs the checkout's whole suite, each unit in a process of its own. It can take many
  minutes; call it once, wait for its answer, and read its log.

Both answer a command run as `testbed_run` does; the suite's `testRunId` reads back with `testbed_test_run`.

## Seeing the machine

- `testbed_queues` shows each lane: its `width`, its `holders`, one for each seat held, and who is
  `waiting`, in the order they will be served, with `front` and `next` marking those who stand ahead of
  the ordinary line, and the runs this machine `placed` on another, each naming its `machine`, with
  `heldAt` null while it waits there.
- `testbed_test_runs` pages the test runs, newest first; `checkout` narrows them to one checkout, and the
  `id` of the last one is the `after` of the next page while `more` is true.
- `testbed_deployments` and `testbed_deployment` read the deployments the testbed ran: their `state`, each
  step with its exit code and times, and `stoppedFor`, the stage a deployment stopped in where it did not go
  live.

## Putting a commit live

The `deploy` verb refuses a harness's shell. A session the tool host serves puts a commit live with the
hosted tool `testbed/deploy` instead, through `bmo use`, naming the repository, the commit, already pushed,
and why in English. It waits for the deployment to end and answers `live:` with the commit, or an error with
the deployment's journal; `--async` leaves the turn free and wakes it with the same answer.

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
