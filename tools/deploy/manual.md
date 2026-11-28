# testbed/deploy

Put a pushed commit of a repository live on this machine and answer whether it went live. A job never deploys:
its close pushes its commit onto main and its item waits for its repository's batch deployment. Putting it live
is yours, with this tool and in no other way: never through the shell's deploy verb, which refuses an agent's
shell, and never by hand.

## The call

- `bmo use testbed/deploy --async --room <room> --agent <you> --json '{"repository": "bmo-job", "commit": "<main's
  commit>", "why": "Batch carrying #4100 and #4101"}'`. Call it with `--async`: a deployment takes minutes, and its
  outcome wakes you in the room.
- `repository` is the name the checkout's testbed.toml declares. `commit` is main's commit, pushed. `why` is English
  and names the items the batch carries.
- A deployment of the same commit already under way is followed, not started again.
- `first` with an English reason puts an urgent deployment ahead of the line's waiting ones.
- `dry_run: true` says what would happen and changes nothing.
- A call the tool host's restart ended: the deployment goes on. Call again with `attach: true` to read its
  outcome.
- The answer's first line begins `live:` for a deployment that went live. Anything else is marked `isError` true
  and says why.

## When to deploy

- Read the deployments already waiting first. A repository is never queued twice: its newest commit carries the
  older ones, so only the newest goes.
- Each time a job ends, and each time you are woken, read with `job_items` the items whose label
  `close-phase:<repository>` reads `waiting-batch` (label filter `close-phase:<repository>=waiting-batch`), and the
  older ones whose bare `close-phase` does. For each such repository, look for related work still open or leased:
  an item of the same repository, one naming it in its `repositories` label, or one of the same `series` in another
  repository.
- While any is left, the commits wait. When none is left, or the owner says now, put that repository's main live
  once, without asking anybody: every item that runs was approved, and that approval covers putting its commit
  live. A `series` that spans several repositories goes live together, each repository's main in turn.
- Agency flushes the whole deploy queue every hour by itself, related work open or not. You never deploy for the
  flush and never wait for it; its outcome comes to you as `deploy-live` or `deploy-failed`.
- Only the owner's own hold on an item keeps you from putting its commit live. Then say that the commit waits and
  that the hourly flush puts it live. Never ask the owner whether to put live a commit you may put live.

## After it ends

- Went live: for each item that waited on that repository, take its pair off with `job_item_update`, the labels
  `close-phase:<repository>` and `close-commit:<repository>` set to null. An item that still carries another
  repository's pair reading `waiting-batch` keeps waiting for that batch; close every other item with
  `job_item_update` and `state` `done`. Say it once in the room: the repository, the commit that went live, the items
  closed and the items still waiting.
- Did not go live: read the failing step, its last lines and the commits the batch carried, and name the commit that
  broke it: the one whose files the failing step reads, else the newest. Send that item back: take its pair off,
  put what failed on it with `job_note_add`, and launch it again. The other items wait for the next batch, which goes
  out once the mend is merged.
- A job under review that waits on its own commit going live: put the commit live while the job stays under review,
  then send the review back naming the stage that holds the check, saying the commit is live.

## Adding a repository to the live machine

- `bmo install --add <repository>` puts a new service live beside the running ones, so no item's approval or
  standing word covers it. Ask the owner, naming the repository and what it turns on, and run it only once he has
  approved that repository in his own words; a later repository is asked again. A job or a worker never runs it:
  a job that needs a repository added asks you, and you put that question to him.

## A change that is live

- A deployment that changes how the system stands (a service retired or added, a card or node reassigned, a model
  replaced, a unit's role changed) is written to your memories in the same turn: read the rows it bears on with
  `bmo/brain_similar` and replace them with the new version, the change and the moment it took effect.
