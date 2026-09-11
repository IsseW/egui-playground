//! Binary encoding of what the host sends into a guest program and what comes back.
//!
//! The host runs as `wasm32-unknown-unknown` and the guest as `wasm32-wasip1`, so they cannot
//! share Rust types. Both link this crate and exchange the byte buffers it produces.
//!
//! Both encodings start with [`MAGIC`] and [`VERSION`]. Every length is a little-endian `u32`
//! and every number is little-endian.

mod buf;
mod input;
mod output;

#[cfg(test)]
mod tests;

pub use buf::{Reader, Writer};
pub use input::{decode_input, encode_input, Input};
pub use output::{decode_output, encode_output, FrameOutput};

/// First four bytes of every buffer, "egui playground bridge".
pub const MAGIC: [u8; 4] = *b"EPGB";

/// Bumped whenever an encoding changes. Host and guest must agree.
pub const VERSION: u16 = 1;

/// Why a buffer could not be decoded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DecodeError {
    /// The buffer does not start with [`MAGIC`].
    BadMagic,

    /// The buffer was written with a different [`VERSION`].
    VersionMismatch { found: u16 },

    /// The buffer ended in the middle of a value.
    UnexpectedEnd,

    /// A string field was not valid UTF-8.
    InvalidUtf8,

    /// An enum tag outside the range this version knows.
    BadTag { what: &'static str, tag: u32 },

    /// A key name that this egui version does not have.
    UnknownKey,
}

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BadMagic => write!(f, "not a playground bridge buffer"),
            Self::VersionMismatch { found } => {
                write!(f, "bridge version {found}, expected {VERSION}")
            }
            Self::UnexpectedEnd => write!(f, "buffer ended mid-value"),
            Self::InvalidUtf8 => write!(f, "string field was not valid UTF-8"),
            Self::BadTag { what, tag } => write!(f, "unknown {what} tag {tag}"),
            Self::UnknownKey => write!(f, "unknown key name"),
        }
    }
}

impl std::error::Error for DecodeError {}

pub(crate) fn write_header(w: &mut Writer<'_>) {
    w.raw(&MAGIC);
    w.u16(VERSION);
}

pub(crate) fn read_header(r: &mut Reader<'_>) -> Result<(), DecodeError> {
    if r.raw(MAGIC.len())? != MAGIC {
        return Err(DecodeError::BadMagic);
    }
    let version = r.u16()?;
    if version != VERSION {
        return Err(DecodeError::VersionMismatch { found: version });
    }
    Ok(())
}
