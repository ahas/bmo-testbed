//! What every Rust test shares, the twin of the test kit's `ids` (docs/boundary.md, 「The crates」): the
//! fixed ids a test names a user, a room, a message, an account, a worker and a unit by, the same values
//! the TypeScript tests read, kept in one place. A test needing a second one of a kind names its own beside
//! these.

/// A user's id, which is also the chat id of that user's private chat.
pub const OWNER: i64 = 42;
/// A group room's chat id.
pub const ROOM: i64 = -100;
/// A message's id within its room.
pub const MESSAGE: i64 = 9;
/// An account's id.
pub const ACCOUNT: &str = "account";
/// A worker's name: the built-in worker's.
pub const WORKER: &str = "worker";
/// A unit's name.
pub const UNIT: &str = "bmo-task-1";
