//! Back-compat shim: the session vocabulary now lives at `crate::session`
//! (the anti-corrosion layer shared by every driver). Re-exported here so the
//! historical `crate::claude::session::*` import paths keep compiling.

pub use crate::session::*;
