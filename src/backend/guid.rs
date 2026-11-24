//! Globally unique identifiers for everything the daemon creates (sessions, windows,
//! terminals and runs of sysd sessions).
//!
//! IDs are never reused: an event that arrives late for something that is already gone
//! (a tracked command's exit, a waiter for an old run) can never be mistaken for
//! something new that happens to have been handed the same ID.

use std::collections::HashSet;
use std::fmt;

use bincode_next::{Decode, Encode};
use serde::{Deserialize, Serialize};

/// A random (version 4) UUID.
#[derive(Clone, Copy, Eq, Hash, PartialEq, Ord, PartialOrd, Encode, Decode, Serialize, Deserialize)]
pub struct Guid(u128);

impl Guid {
    /// The all-zero GUID. `GuidManager` never generates it, so it can stand for "none".
    pub const NIL: Guid = Guid(0);

    const VERSION_MASK: u128 = 0xF << 76;
    const VERSION_4: u128 = 0x4 << 76;
    const VARIANT_MASK: u128 = 0b11 << 62;
    const VARIANT_RFC4122: u128 = 0b10 << 62;

    /// A fixed GUID, for tests and placeholders. Generated GUIDs always carry the
    /// version 4 bits, so small values like `from_u128(1)` never clash with them.
    pub const fn from_u128(value: u128) -> Self {
        Guid(value)
    }

    pub const fn as_u128(self) -> u128 {
        self.0
    }

    /// 122 random bits with the version and variant bits set as RFC 9562 requires.
    fn random() -> Self {
        let bits: u128 = rand::random();
        Guid((bits & !Self::VERSION_MASK & !Self::VARIANT_MASK) | Self::VERSION_4 | Self::VARIANT_RFC4122)
    }
}

impl fmt::Display for Guid {
    /// The usual `xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx` form.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let v = self.0;
        write!(
            f,
            "{:08x}-{:04x}-{:04x}-{:04x}-{:012x}",
            v >> 96,
            (v >> 80) & 0xFFFF,
            (v >> 64) & 0xFFFF,
            (v >> 48) & 0xFFFF,
            v & 0xFFFF_FFFF_FFFF,
        )
    }
}

impl fmt::Debug for Guid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

/// Hands out GUIDs that are guaranteed to be unique among everything this manager has
/// ever issued.
///
/// GUIDs come from the thread-local CSPRNG, so a collision is already astronomically
/// unlikely; the manager additionally remembers every GUID it issued and draws again on
/// a clash, which turns "unlikely" into "impossible". GUIDs are never released, so one is
/// never handed out twice, not even after the thing it named is gone. Remembering them
/// costs a few dozen bytes per ID, which is nothing for the number of sessions,
/// windows and terminals a daemon creates.
#[derive(Default)]
pub struct GuidManager {
    issued: HashSet<Guid>,
}

impl GuidManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// A GUID this manager has never issued before.
    pub fn generate(&mut self) -> Guid {
        loop {
            let guid = Guid::random();
            // False if this GUID was issued before: draw again.
            if self.issued.insert(guid) {
                return guid;
            }
        }
    }
}
