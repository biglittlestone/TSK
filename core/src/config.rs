//! Three-layer configuration.
//!
//! Layer 1 (user): `~/.tsk/config.yaml` — two keys only. See spec §2.1.
//! Layer 2 (engineering): compile-time constants below. See spec §2.2.
//! Layer 3 (debug): `TSK_ADV_<CONST>` env vars temporarily override a constant.

use serde::{Deserialize, Serialize};
use std::path::Path;

/// The only genuine user decision. The single non-lossless knob: it changes
/// reply *style*, never technical correctness. Default `off` (opt-in).
/// `auto` defers the decision to the session: skip injection while the session
/// is too short for output compression to pay back its fixed ruleset cost.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CompressionLevel {
    #[default]
    Off,
    Lite,
    Full,
    Ultra,
    Auto,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// One-key global off (diagnostics safety valve).
    pub enabled: bool,
    /// Output-style compression. `Off` ⇒ no injection, ever.
    pub output_compression: CompressionLevel,
}

impl Config {
    /// Compression active only when the kill switch is up and a level is chosen.
    pub fn compression_on(&self) -> bool {
        self.enabled && self.output_compression != CompressionLevel::Off
    }

    /// Read `path`. Missing file ⇒ the default (enabled, `Off`).
    pub fn load(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(s) => serde_yaml::from_str(&s).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    /// YAML exactly as `tsk init` writes it. `output_compression` is the only
    /// real user decision; the comment documents every level including `auto`.
    pub fn default_yaml() -> &'static str {
        "enabled: true\n\
         # output_compression: the only real user decision (changes reply STYLE, never correctness).\n\
         #   off   = no output compression (default)\n\
         #   lite  = short ruleset, low injection cost (~361 tok/session), weaker effect\n\
         #   full  = full ruleset, higher injection (~967 tok/session), strongest effect (-68.8% measured)\n\
         #   ultra = same as full (reserved for a sharper level)\n\
         #   auto  = auto on/off by session length: skip injection for the first 2 turns\n\
         #           (zero cost for short sessions), start injecting from turn 3 (break-even≈3)\n\
         # This option changes reply STYLE (user-visible: replies get terser), but\n\
         # keeps technical CORRECTNESS lossless (code/commands/paths verbatim,\n\
         # AUTO-CLARITY safety exception). Input & history compression are lossless.\n\
         output_compression: off\n"
    }

    /// The current settings serialized, e.g. for `tsk off` to persist `enabled: false`.
    pub fn to_yaml(&self) -> Result<String, Box<dyn std::error::Error>> {
        Ok(serde_yaml::to_string(self)?)
    }
}

/// Head lines kept in a skeleton. (spec §2.2)
pub const SKELETON_HEAD: usize = 5;
/// Tail lines kept in a skeleton.
pub const SKELETON_TAIL: usize = 3;
/// Files with fewer lines than this are not compressed (head+tail would overlap).
pub const MIN_LINES: usize = 8;
/// Below this size, a `Read` goes straight to the skeleton path.
pub const ROUTE_THRESHOLD: usize = 50 * 1024;
/// Above this size, a `Read` result is externalized to a pointer + excerpt.
pub const EXTERNALIZE_THRESHOLD: usize = 100 * 1024;
/// Max sandbox stdout; beyond this it is summarized and externalized.
pub const SANDBOX_MAX_OUTPUT: usize = 64 * 1024;
/// Inputs larger than this are refused outright.
pub const SANDBOX_HARD_CAP: u64 = 100 * 1024 * 1024;
/// Sandbox wall-clock budget.
pub const SANDBOX_TIMEOUT: u64 = 10;
/// Injection budget in estimated tokens.
pub const INJECT_BUDGET: usize = 500;
/// P1 (role) is never truncated; it is capped at this many chars.
pub const INJECT_P1_MAX_CHARS: usize = 400;
/// Max decisions (P2) kept when over budget, before falling back to 3.
pub const INJECT_P2_N: usize = 5;
/// Max skills (P3) kept.
pub const INJECT_P3_N: usize = 10;
/// Hard ceiling on the resume snapshot on disk.
pub const SNAPSHOT_MAX: usize = 2 * 1024;
/// Above this last-turn input usage, suggest `/compact`.
pub const COMPACT_ADVICE_THRESHOLD: usize = 180_000;
/// Ruleset below this length is diluted by models (spike S3).
pub const RULESET_MIN_CHARS: usize = 3500;
