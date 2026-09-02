//! Central tag allocation. Every action sector gets a unique nonzero tag.

use crate::compile::{CompileError, MapData};

/// One allocation, recorded for the run's tag manifest.
#[derive(Debug, Clone)]
pub struct TagEntry {
    /// The allocated tag.
    pub tag: u16,
    /// The sector it identifies.
    pub sector: usize,
    /// Why it was allocated, for human audit.
    pub purpose: String,
}

/// Hands out unique, nonzero sector tags and records what each is for.
#[derive(Debug, Default)]
pub struct TagAllocator {
    next: u16,
    manifest: Vec<TagEntry>,
}

impl TagAllocator {
    /// Creates an allocator whose first tag will be 1.
    #[must_use]
    pub fn new() -> Self {
        Self {
            next: 0,
            manifest: Vec::new(),
        }
    }

    /// Allocates the next unused tag for a sector.
    ///
    /// # Panics
    /// Panics if more than `u16::MAX` tags are requested, which no map of the
    /// sizes this compiler targets can reach.
    pub fn allocate(&mut self, sector: usize, purpose: &str) -> u16 {
        self.next = self.next.checked_add(1).expect("tag space exhausted");
        self.manifest.push(TagEntry {
            tag: self.next,
            sector,
            purpose: purpose.to_owned(),
        });
        self.next
    }

    /// Points an already-allocated tag at the sector it ended up on.
    ///
    /// For a caller that must allocate before its sector exists:
    /// [`crate::compile::floors::emit_floors`] takes one tag per trigger up
    /// front, so that every construct the trigger fires can be stamped with
    /// it as it is emitted, and only then knows which sector to record. A
    /// no-op for a tag never allocated; the manifest is an audit record, and
    /// inventing a row for a tag nobody handed out would make it a worse one.
    pub fn rename_sector(&mut self, tag: u16, sector: usize) {
        if let Some(entry) = self.manifest.iter_mut().find(|e| e.tag == tag) {
            entry.sector = sector;
        }
    }

    /// Every allocation made so far, in order.
    #[must_use]
    pub fn manifest(&self) -> &[TagEntry] {
        &self.manifest
    }
}

/// Rejects any linedef that carries a special but no tag.
///
/// # Errors
/// Returns [`CompileError::ActionAtTagZero`] naming the first offending line.
pub fn check_no_action_at_tag_zero(data: &MapData) -> Result<(), CompileError> {
    for (index, line) in data.linedefs.iter().enumerate() {
        if line.special != 0 && line.tag == 0 {
            return Err(CompileError::ActionAtTagZero {
                index,
                special: line.special,
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{TagAllocator, check_no_action_at_tag_zero};
    use crate::compile::{CompileError, LinedefOut, MapData, SectorOut, SidedefOut};

    #[test]
    fn allocation_starts_at_one_and_never_repeats() {
        let mut alloc = TagAllocator::new();
        let a = alloc.allocate(0, "door a");
        let b = alloc.allocate(1, "door b");
        let c = alloc.allocate(0, "lift on the same sector");
        assert_eq!(a, 1, "tags start at 1, never 0");
        assert!(a != b && b != c && a != c, "tags never repeat");
        assert_eq!(alloc.manifest().len(), 3, "every allocation is recorded");
        assert_eq!(alloc.manifest()[2].purpose, "lift on the same sector");
    }

    #[test]
    fn renaming_points_a_tag_at_the_sector_it_ended_up_on() {
        let mut alloc = TagAllocator::new();
        let first = alloc.allocate(usize::MAX, "trigger t: drop wall a <-> b");
        let second = alloc.allocate(4, "door a");
        alloc.rename_sector(first, 7);
        alloc.rename_sector(second + 1, 9);
        assert_eq!(
            alloc.manifest()[0].sector,
            7,
            "the placeholder is replaced by the real sector"
        );
        assert_eq!(
            alloc.manifest()[0].purpose,
            "trigger t: drop wall a <-> b",
            "nothing else about the entry changes"
        );
        assert_eq!(alloc.manifest()[1].sector, 4, "no other entry moves");
        assert_eq!(
            alloc.manifest().len(),
            2,
            "renaming a tag nobody allocated adds no row"
        );
    }

    fn map_with_line(special: u16, tag: u16) -> MapData {
        MapData {
            vertices: vec![],
            sectors: vec![SectorOut {
                floor: 0,
                ceiling: 128,
                light: 160,
                floor_tex: "F".into(),
                ceil_tex: "C".into(),
                special: 0,
                tag: 0,
                wall_tex: String::new(),
                host: None,
            }],
            sidedefs: vec![SidedefOut {
                sector: 0,
                upper: String::new(),
                middle: String::new(),
                lower: String::new(),
                x_offset: 0,
            }],
            linedefs: vec![LinedefOut {
                v1: 0,
                v2: 1,
                front: 0,
                back: None,
                blocking: true,
                special,
                tag,
                lower_unpegged: false,
                upper_unpegged: false,
                secret: false,
            }],
        }
    }

    #[test]
    fn an_action_line_at_tag_zero_is_rejected() {
        let data = map_with_line(1, 0);
        assert!(matches!(
            check_no_action_at_tag_zero(&data),
            Err(CompileError::ActionAtTagZero { .. })
        ));
    }

    #[test]
    fn an_action_line_with_a_real_tag_passes() {
        assert!(check_no_action_at_tag_zero(&map_with_line(1, 7)).is_ok());
    }

    #[test]
    fn a_plain_line_at_tag_zero_is_fine() {
        assert!(check_no_action_at_tag_zero(&map_with_line(0, 0)).is_ok());
    }
}
