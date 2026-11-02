//! What the backup's tests stand on: an S3 bucket and an rclone remote stood in for by programs that keep
//! their objects in a folder, and a plan over a root of the test's own.

use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};

use bmo_testbed_backup::settings::{Plan, Settings};

// The AWS command's words the store uses, answered over a folder: one folder per bucket, a key its path.
const AWS: &str = r#"#!/bin/sh
root='@ROOT@'
while [ "$1" = --profile ] || [ "$1" = --endpoint-url ]; do shift 2; done
case "$1 $2" in
  "s3api list-objects-v2")
    shift 2; bucket=; prefix=
    while [ $# -gt 0 ]; do case "$1" in --bucket) bucket=$2; shift 2;; --prefix) prefix=$2; shift 2;; *) shift;; esac; done
    [ -d "$root/$bucket" ] || { echo "no such bucket" >&2; exit 254; }
    find "$root/$bucket" -type f -printf '%P\t%s\n' | awk -F'\t' -v p="$prefix" '
      BEGIN { printf "{\"Contents\": ["; n = 0 }
      index($1, p) == 1 { if (n++) printf ","; printf "{\"Key\": \"%s\", \"Size\": %s}", $1, $2 }
      END { print "]}" }'
    ;;
  "s3 cp")
    from=$3; to=$4
    case "$from" in
      s3://*) cp "$root/${from#s3://}" "$to" ;;
      *) mkdir -p "$(dirname "$root/${to#s3://}")" && cp "$from" "$root/${to#s3://}" ;;
    esac
    ;;
  "s3 rm") rm -f "$root/${3#s3://}" ;;
  *) echo "unknown: $*" >&2; exit 2 ;;
esac
"#;

// The rclone words the store uses, answered over a folder: `remote:` the folder itself.
const RCLONE: &str = r#"#!/bin/sh
root='@ROOT@'
place() { echo "$root/${1#*:}"; }
case "$1" in
  lsjson)
    for last; do :; done
    dir=$(place "$last")
    [ -d "$dir" ] || { echo "directory not found" >&2; exit 3; }
    find "$dir" -type f -printf '%P\t%s\n' | awk -F'\t' '
      BEGIN { printf "["; n = 0 }
      { if (n++) printf ","; printf "{\"Path\": \"%s\", \"Size\": %s}", $1, $2 }
      END { print "]" }'
    ;;
  copyto)
    case "$2" in
      *:*) cp "$(place "$2")" "$3" ;;
      *) mkdir -p "$(dirname "$(place "$3")")" && cp "$2" "$(place "$3")" ;;
    esac
    ;;
  deletefile) rm "$(place "$2")" ;;
  *) echo "unknown: $*" >&2; exit 2 ;;
esac
"#;

/// A program written into a folder, its root filled in.
fn program(dir: &Path, name: &str, text: &str, root: &Path) -> PathBuf {
  let path = dir.join(name);

  std::fs::write(&path, text.replace("@ROOT@", &root.to_string_lossy())).unwrap();
  std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();

  // A process another test forks while the file is still open for writing holds it open until it execs,
  // and the program cannot be started meanwhile; it is started once here until it can be.
  while std::process::Command::new(&path)
    .stderr(std::process::Stdio::null())
    .status()
    .is_err_and(|why| why.raw_os_error() == Some(libc::ETXTBSY))
  {
    std::thread::sleep(std::time::Duration::from_millis(5));
  }

  path
}

/// A test's own ground: the root backed up, the staging folder, and the stand-ins' programs and stores.
pub struct Ground {
  pub dir: tempfile::TempDir,
}

impl Ground {
  pub fn new() -> Self {
    let ground = Self {
      dir: tempfile::tempdir().unwrap(),
    };

    for one in ["root", "bin", "s3/bucket", "remote"] {
      std::fs::create_dir_all(ground.path(one)).unwrap();
    }

    program(&ground.path("bin"), "aws", AWS, &ground.path("s3"));
    program(&ground.path("bin"), "rclone", RCLONE, &ground.path("remote"));
    ground
  }

  pub fn path(&self, under: &str) -> PathBuf {
    self.dir.path().join(under)
  }

  /// A file written under the root, its folders made.
  pub fn file(&self, path: &str, bytes: usize) {
    let path = self.path("root").join(path);

    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, vec![b'x'; bytes]).unwrap();
  }

  /// Settings over this ground with both stand-ins as destinations, the body's own keys ahead of them.
  pub fn settings(&self, body: &str) -> String {
    format!(
      "root = \"{root}\"\nstaging = \"{staging}\"\n{body}\n\n\
       [[destination]]\nname = \"bucket\"\nkind = \"s3\"\nbucket = \"bucket\"\nprofile = \"stand-in\"\ncommand = \"{aws}\"\n\n\
       [[destination]]\nname = \"remote\"\nkind = \"rclone\"\nremote = \"remote:kept\"\ncommand = \"{rclone}\"\n",
      root = self.path("root").display(),
      staging = self.path("staging").display(),
      aws = self.path("bin/aws").display(),
      rclone = self.path("bin/rclone").display(),
    )
  }

  /// The plan of settings text.
  pub fn plan(&self, text: &str) -> Plan {
    let settings: Settings = toml::from_str(text).unwrap();

    Plan::of(settings, self.dir.path()).unwrap_or_else(|why| panic!("{why}"))
  }

  /// The objects standing in the stand-in bucket, by key.
  pub fn in_bucket(&self) -> Vec<String> {
    keys_under(&self.path("s3/bucket"))
  }

  /// The objects standing on the stand-in remote, by path under its folder.
  pub fn on_remote(&self) -> Vec<String> {
    keys_under(&self.path("remote/kept"))
  }
}

fn keys_under(dir: &Path) -> Vec<String> {
  let mut keys = Vec::new();
  let mut stack = vec![dir.to_path_buf()];

  while let Some(next) = stack.pop() {
    for entry in std::fs::read_dir(&next).into_iter().flatten().flatten() {
      let path = entry.path();

      if path.is_dir() {
        stack.push(path);
      } else {
        keys.push(path.strip_prefix(dir).unwrap().to_string_lossy().into_owned());
      }
    }
  }

  keys.sort();
  keys
}
