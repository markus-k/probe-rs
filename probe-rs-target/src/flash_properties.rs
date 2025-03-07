use super::memory::SectorDescription;
use serde::{Deserialize, Serialize};
use std::ops::Range;

/// Properties of flash memory, which
/// are used when programming Flash memory.
///
/// These values are read from the
/// YAML target description files.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct FlashProperties {
    /// The range of the device flash.
    pub address_range: Range<u32>,
    /// The page size of the device flash.
    pub page_size: u32,
    /// The value of a byte in flash that was just erased.
    pub erased_byte_value: u8,
    /// The approximative time it takes to program a page.
    pub program_page_timeout: u32,
    /// The approximative time it takes to erase a sector.
    pub erase_sector_timeout: u32,
    /// The available sectors of the device flash.
    pub sectors: Vec<SectorDescription>,
}

impl Default for FlashProperties {
    #[allow(clippy::reversed_empty_ranges)]
    fn default() -> Self {
        FlashProperties {
            address_range: 0..0,
            page_size: 0,
            erased_byte_value: 0,
            program_page_timeout: 0,
            erase_sector_timeout: 0,
            sectors: vec![],
        }
    }
}
