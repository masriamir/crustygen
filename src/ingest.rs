//! Shared WAD-map ingestion for the layer-4 tools (issue #21).
//!
//! [`load_map`] turns one of a WAD's map groups into the parsed [`UdmfMap`]
//! the checker and lifter consume, regardless of on-disk format:
//!
//! - A UDMF group (has a `TEXTMAP` lump): the lump's text parses directly.
//! - A classic Doom binary group: assembled via [`Map::assemble`] (which
//!   normalizes binary maps), rendered to UDMF text via `write_udmf`, and
//!   re-parsed. Binary Doom maps carry doom-namespace semantics — the same
//!   special/tag numbering every check models — so nothing about the
//!   catalog becomes a lie for them (issue #21).
//! - Hexen and Doom 64 groups are refused: Hexen-style specials are a
//!   different numbering the checks do not model.
//!
//! The round trip is labeled: [`LoadedMap::origin`] records which path ran,
//! so CLI output can say a map was assembled from binary format.

use crustywad::map::udmf::{
    UdmfMap, UdmfParseError, UdmfWriteError, UdmfWriteWarning, parse_udmf, write_udmf,
};
use crustywad::map::{Map, MapAssembleError, MapFormat, MapGroup};
use crustywad::{Limits, Wad, WriteOptions};

/// Which ingestion path produced a [`LoadedMap`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapOrigin {
    /// The group carried a `TEXTMAP` lump, parsed directly.
    Udmf,
    /// The group was a classic Doom binary map: assembled, rendered to UDMF
    /// text, and re-parsed.
    AssembledFromBinary,
}

/// A parsed map plus how it was obtained and any render-path notes.
#[derive(Debug)]
pub struct LoadedMap {
    /// The parsed UDMF document.
    pub map: UdmfMap,
    /// Which path produced it.
    pub origin: MapOrigin,
    /// Human-readable warnings from the binary render path. The expected
    /// `NamespaceDefaulted` warning (inherent to every binary map) is
    /// filtered out; anything else is preserved for the CLI to surface.
    pub notes: Vec<String>,
}

/// A failure to turn a map group into a parsed [`UdmfMap`].
#[derive(Debug, thiserror::Error)]
pub enum IngestError {
    /// The group's `TEXTMAP` lump is not valid UTF-8.
    #[error("TEXTMAP lump is not valid UTF-8: {0}")]
    NonUtf8Textmap(std::str::Utf8Error),
    /// The group's `TEXTMAP` text failed to parse as UDMF.
    #[error("failed to parse TEXTMAP: {0}")]
    UdmfParse(#[from] UdmfParseError),
    /// A binary-format group failed strict assembly.
    #[error("failed to assemble binary map: {0}")]
    Assemble(#[from] MapAssembleError),
    /// The assembled binary map is not the classic Doom format.
    #[error(
        "unsupported binary map format {format:?}: only classic Doom-format \
         binary maps are supported (Hexen-style specials are a different \
         numbering the checks do not model)"
    )]
    UnsupportedBinaryFormat {
        /// The refused format.
        format: MapFormat,
    },
    /// Rendering the assembled map to UDMF text failed.
    #[error("failed to render assembled map as UDMF: {0}")]
    Render(#[from] UdmfWriteError),
    /// The rendered UDMF text failed to re-parse.
    #[error("round-trip UDMF failed to re-parse: {0}")]
    Reparse(UdmfParseError),
}

/// Loads `group` from `wad` as a parsed [`UdmfMap`] via whichever path its
/// on-disk format requires (see the module docs).
///
/// # Errors
/// Returns [`IngestError`] naming the first failure: a non-UTF-8 or
/// unparseable `TEXTMAP`, a binary group that fails strict assembly, a
/// non-Doom binary format, a render failure, or a re-parse failure.
pub fn load_map(wad: &Wad, group: &MapGroup) -> Result<LoadedMap, IngestError> {
    let lumps = wad.lumps();
    let textmap = group
        .data_indices
        .iter()
        .copied()
        .find(|&i| lumps[i].name() == "TEXTMAP");
    if let Some(idx) = textmap {
        let text =
            std::str::from_utf8(wad.lump_data(&lumps[idx])).map_err(IngestError::NonUtf8Textmap)?;
        let map = parse_udmf(text, Limits::default())?;
        return Ok(LoadedMap {
            map,
            origin: MapOrigin::Udmf,
            notes: Vec::new(),
        });
    }

    let assembled = Map::assemble(wad, group)?;
    ensure_doom_format(assembled.format())?;
    let (text, warnings) = write_udmf(&assembled, &WriteOptions::default())?;
    let notes = warnings
        .iter()
        .filter(|w| !matches!(w, UdmfWriteWarning::NamespaceDefaulted { .. }))
        .map(ToString::to_string)
        .collect();
    // COVERAGE: Reparse is defensive — crustywad's own writer emits this
    // text, and its round-trip guarantee makes a re-parse failure a
    // crustywad bug, not a reachable input.
    let map = parse_udmf(&text, Limits::default()).map_err(IngestError::Reparse)?;
    Ok(LoadedMap {
        map,
        origin: MapOrigin::AssembledFromBinary,
        notes,
    })
}

/// Accepts only [`MapFormat::Doom`] on the binary path.
fn ensure_doom_format(format: MapFormat) -> Result<(), IngestError> {
    match format {
        MapFormat::Doom => Ok(()),
        other => Err(IngestError::UnsupportedBinaryFormat { format: other }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doom_format_passes_the_gate() {
        assert!(ensure_doom_format(MapFormat::Doom).is_ok());
    }

    #[test]
    fn hexen_and_doom64_are_refused_by_name() {
        for format in [MapFormat::Hexen, MapFormat::Doom64, MapFormat::Udmf] {
            let err = ensure_doom_format(format).expect_err("must refuse");
            let msg = err.to_string();
            assert!(msg.contains("unsupported binary map format"), "got: {msg}");
        }
    }
}
