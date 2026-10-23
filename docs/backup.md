# The data backup: a machine's databases and data folders, kept at its destinations

## What it is

- **The `backup` verb keeps a machine's runtime data restorable,** apart from any deployment: every hour
  a dump of each database it is told of and one archive of the data folders it is told of, sent to every
  destination it is told of and rotated there; every week the large media, each as an object of its own;
  every day a restore drill that proves the newest copy restores.
- **What it backs up is the settings file's alone.** The testbed names no database, no folder and no
  destination of anyone's; the machine names them in one TOML file, `<data root>/env/backup.toml`, the
  data root being the testbed's own (`docs/boundary.md`, 「The environment」). The numbers it rotates by
  have defaults in the testbed's policy, which the file may set otherwise.
- **The words:** `backup snapshot`, `backup media`, `backup drill` and `backup prune [--dry-run]`, each
  with a timer of its own in this repository's `units/`, kept by every deployment of it (`docs/deploy.md`,
  「The unit links」).

## What is backed up

| Set | What it holds | How often | Where it stands at a destination |
|---|---|---|---|
| Hourly | One custom-format dump of each database, `<database>.pgdump`; one archive of the data folders, `data.tar.zst`; the manifest, `manifest.json` | Every hour | `<prefix>/hourly/<slot>/`, the slot the hour falls in |
| Daily | The day's first hourly snapshot, whole | Once a day | `<prefix>/daily/<YYYY-MM-DD>/` |
| Media | Each declared media set as one archive, and every file over the size threshold inside the data folders as one more, `large-files` | Every week | `<prefix>/media/<name>.tar.zst`, the same name every week |
| Releases | The newest release archives of each declared release folder | Every hour, where a new one stands | `<prefix>/deploy/<folder>/<archive>` |

- **The databases** are every database on the server whose name one of `databases` matches and none of
  `exclude_databases` does, and never one of the drill's own scratch databases. Each is dumped by
  `pg_dump` in custom format, compressed as the policy says, through the server's address with its own
  name in place of the listing database's, or through the address the file `address_files` names for it
  holds, where a database is reached as a role of its own.
- **A dump and its row counts are one moment.** Before each dump a transaction is opened at repeatable
  read and its snapshot exported; `pg_dump` dumps that snapshot while the same transaction counts every
  table's rows, so the manifest's counts are the dump's exactly. A table an extension owns is left out
  of the counts, since `pg_dump` dumps no rows of it.
- **The data archive** is the declared `folders` under the declared `root`, packed as one tar stream
  through `zstd`, leaving out:
  - every path an `exclude` glob matches, read from the root;
  - every media set's folders;
  - every file over `large_file_bytes`, which goes to the weekly `large-files` archive instead.
  A file that changes while tar reads it goes in as it stood.
- **The manifest** says when the snapshot was taken and which slot it is, and for each file its name,
  size and SHA-256; for each database its tables and their row counts; for the data archive how many
  entries it holds. It names no row and no file's contents.
- **Media** is everything the hourly archive leaves out on purpose: each `[[media]]` set's folders as one
  archive under the set's name, and the large files.

## How a snapshot runs

1. **Take the lock,** one snapshot at a time. A snapshot that finds another going ends at once with exit
   code 75; the next hour's runs as usual.
2. **Dump and pack** into `<staging>/next/`, then put it in place of `<staging>/latest/`, so the newest
   whole snapshot always stands on the local disk.
3. **Send it to each destination in turn,** the ones that take the hourly set. For each:
   1. **Check the room,** where the destination names the disk its store writes to: the disk must keep
      `room_floor_bytes` free after the snapshot lands, or nothing is sent there.
   2. **Take the slot's manifest away first,** so a slot half written is never taken for a whole one.
   3. **Upload each file,** then list it back and hold its size to the local file's; an S3 object also
      carries its SHA-256 as metadata.
   4. **Upload the manifest last,** then take away whatever else the slot held.
   5. **The daily copy:** where the destination takes the daily set and holds no manifest under today's
      date, the same files go there, the manifest last.
   6. **The release archives,** where the destination takes them: each declared folder's newest
      `releases_kept` archives sent where an object of their name and size does not already stand.
   7. **The rotation** (「What is removed」).
4. **A destination that fails is said and left;** the others are still sent. The run ends 1 where any
   step failed, with a `data-backup-failed` event naming each.

- **The slot** is the hour since the epoch modulo `hourly_slots`, so the slots hold the newest hours, one
  each, and every hour overwrites the oldest.
- **The day** is the machine's local date.

## How the media run runs

- **`backup media`, weekly, with a lock of its own,** so the hourly snapshot goes on beside it.
- **Each archive is packed into `<staging>/media/`, sent to every destination that takes the media set,
  listed back and held to its size, and removed locally after;** the object's name stays the same, so
  each week's replaces the last. The room check holds as it does for the snapshot. A send that failed is
  made again by the next week's run.

## What is removed

- **Only what the rotation names, and only under the prefix.** At a destination, nothing outside
  `<prefix>/` is ever touched, and inside it only keys of the shapes above:
  - `hourly/<slot>/…` whose slot is `hourly_slots` or more;
  - `daily/<date>/…` whose date is `daily_days` days before today or older;
  - `deploy/<folder>/<archive>` past each folder's newest `releases_kept`;
  - `media/<name>.tar.zst` whose name no media set declares any more, `large-files` aside.
- **Locally,** `<staging>/next/` left by a run cut off, the drill's downloads, and each media archive once
  sent; never anything under the declared root. Every local removal goes through the deployment's guarded
  removal (`docs/deploy.md`, 「What it refuses」), the root standing as the data root it guards.
- **`backup prune`** runs the rotation alone; **`--dry-run`** says what it would remove and removes
  nothing.

## The restore drill

- **`backup drill`, daily:** the newest snapshot any destination holds, restored and counted.
  1. Every destination's hourly manifests are read; the newest wins, from the first destination in the
     settings' order holding it. One older than `drill_max_age_hours` fails the drill.
  2. Its files are downloaded into `<staging>/drill/`, and each held to the manifest's size and SHA-256.
  3. Each dump is restored by `pg_restore` into a scratch database of the drill's own on the declared
     server, every table's rows counted and held to the manifest's counts, and the scratch database
     dropped, whatever came of it.
  4. The data archive is read through whole, and its entries held to the manifest's count.
  5. The downloads are removed.
- **It ends 0 where everything matched,** and 1 otherwise with a `restore-drill-failed` event naming each
  mismatch by database, table and the two counts, never a row.

## How to restore by hand

1. **Pick the snapshot:** a slot's or a day's `manifest.json` says when it was taken.
2. **Fetch its files** from a destination: `aws --profile <profile> s3 cp --recursive
   s3://<bucket>/<prefix>/hourly/<slot>/ <dir>/` for an S3 bucket, `rclone copy
   <remote>/<prefix>/hourly/<slot> <dir>` for an rclone remote, and check them with the manifest's
   SHA-256 values.
3. **A database:** `createdb <name>`, then `pg_restore --no-owner --dbname=<name> <dir>/<database>.pgdump`,
   with the database's services stopped where it replaces a live one.
4. **The data folders:** `tar -I zstd -xf <dir>/data.tar.zst -C <root>`, into an empty directory first
   where only part of it is wanted; media the same way from `media/<name>.tar.zst`.

## The settings

`<data root>/env/backup.toml`. A key the file leaves out takes the policy's default, where it has one; a
key the testbed does not know refuses the run.

| Key | Default | What it is |
|---|---|---|
| `root` | none, required | The folder `folders`, `exclude` and the media sets are read from; `~` stands for the home directory |
| `folders` | none, required | The folders and files under `root` the hourly archive holds |
| `exclude` | none | Globs, read from `root`, of paths the archives leave out |
| `large_file_bytes` | 67108864 | A file over this goes to the weekly `large-files` archive instead of the hourly one |
| `databases` | none, required | Globs of the database names to dump |
| `exclude_databases` | none | Globs of database names never dumped |
| `server` | `postgres:///postgres?host=/var/run/postgresql` | The database server, its address with the database to list the others from; the drill restores through it |
| `address_files` | none | A table from a database's name to a file holding the address it is dumped through, so a role and any password stand in that file and never in the settings |
| `staging` | `~/.local/state/bmo-testbed/backup` | The local folder snapshots are made in and the newest kept in |
| `prefix` | `data-backup` | The folder every key at a destination stands under |
| `hourly_slots` | 4 | How many hourly snapshots a destination keeps |
| `daily_days` | 7 | How many days of daily snapshots a destination keeps |
| `releases_kept` | 2 | How many of each release folder's newest archives a destination keeps |
| `drill_max_age_hours` | 3 | How old the newest snapshot may be before the drill fails on it |
| `room_floor_bytes` | 10737418240 | What a destination's disk keeps free after a send, where the destination names its disk |
| `[[media]]` | none | A media set: `name`, and `folders` under `root` |
| `[releases] folders` | none | Globs of the folders holding release archives, `*.tar.zst`; each folder's key is the name of the folder above it |
| `[[destination]]` | none, at least one | Where the sets go, in the order they are sent |

**A destination** takes these keys:

| Key | Default | What it is |
|---|---|---|
| `name` | none, required | What the run calls it |
| `kind` | none, required | `s3`, an S3 bucket through the AWS command; `rclone`, an rclone remote |
| `sets` | every set | Which of `hourly`, `daily`, `media` and `releases` it takes |
| `room` | none | A local path on the disk the destination's store writes to, whose free space is checked |
| `command` | `aws` or `rclone` | The program the destination is reached through |
| `bucket` | none, `s3` only | The bucket |
| `profile` | none, `s3` only | The AWS profile, which holds the credentials and the endpoint |
| `endpoint` | none, `s3` only | The endpoint, where the profile names none |
| `remote` | none, `rclone` only | The remote and the folder under it, `remote:folder` |

- **Credentials are never in the settings:** an S3 destination's stand in its AWS profile, an rclone
  remote's in rclone's own configuration. Nothing the verb says carries one.
