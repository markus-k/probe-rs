use super::chip::Chip;
use super::flash_algorithm::RawFlashAlgorithm;
use jep106::JEP106Code;

use serde::{Deserialize, Serialize};

/// Source of a target description.
///
/// This is used for diagnostics, when
/// an error related to a target description occurs.
#[derive(Clone, Debug, PartialEq)]
pub enum TargetDescriptionSource {
    /// The target description is a generic target description,
    /// which just describes a core type (e.g. M4), without any
    /// flash algorithm or memory description.
    Generic,
    /// The target description is a built-in target description,
    /// which was included into probe-rs at compile time.
    BuiltIn,
    /// The target description was from an external source
    /// during runtime.
    External,
}

/// Type of a supported core.
#[derive(Debug, Copy, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoreType {
    /// ARMv6-M: Cortex M0, M0+, M1
    Armv6m,
    /// ARMv7-A: Cortex A7, A9, A15
    Armv7a,
    /// ARMv7-M: Cortex M3
    Armv7m,
    /// ARMv7e-M: Cortex M4, M7
    Armv7em,
    /// ARMv8-M: Cortex M23, M33
    Armv8m,
    /// RISC-V
    Riscv,
}

impl CoreType {
    /// Returns true if the core type is an ARM Cortex-M
    pub fn is_cortex_m(&self) -> bool {
        matches!(
            self,
            CoreType::Armv6m | CoreType::Armv7em | CoreType::Armv7m | CoreType::Armv8m
        )
    }
}

/// The architecture family of a specific [`CoreType`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Architecture {
    /// An ARM core of one of the specific types [`CoreType::Armv6m`], [`CoreType::Armv7m`], [`CoreType::Armv7em`] or [`CoreType::Armv8m`]
    Arm,
    /// A RISC-V core.
    Riscv,
}

impl CoreType {
    /// Returns the parent architecture family of this core type.
    pub fn architecture(&self) -> Architecture {
        match self {
            CoreType::Riscv => Architecture::Riscv,
            _ => Architecture::Arm,
        }
    }
}

/// Instruction set used by a core
#[derive(Debug, Copy, Clone, PartialEq)]
pub enum InstructionSet {
    /// ARM Thumb 2 instruction set
    Thumb2,
    /// ARM A32 (often just called ARM) instruction set
    A32,
    /// RISC-V 32-bit instruction set
    RV32,
}

/// This describes a chip family with all its variants.
///
/// This struct is usually read from a target description
/// file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChipFamily {
    /// This is the name of the chip family in base form.
    /// E.g. `nRF52832`.
    pub name: String,
    /// The JEP106 code of the manufacturer.
    #[cfg_attr(
        not(feature = "bincode"),
        serde(skip_serializing_if = "Option::is_none")
    )]
    pub manufacturer: Option<JEP106Code>,
    /// This vector holds all the variants of the family.
    pub variants: Vec<Chip>,
    /// This vector holds all available algorithms.
    pub flash_algorithms: Vec<RawFlashAlgorithm>,

    #[serde(skip, default = "default_source")]
    /// Source of the target description, used for diagnostics
    pub source: TargetDescriptionSource,
}

fn default_source() -> TargetDescriptionSource {
    TargetDescriptionSource::External
}

impl ChipFamily {
    /// Validates the [`ChipFamily`] such that probe-rs can make assumptions about the correctness without validating thereafter.
    ///
    /// This method should be called right after the [`ChipFamily`] is created!
    pub fn validate(&self) -> Result<(), String> {
        // We check each variant if it is valid.
        // If one is not valid, we abort with an appropriate error message.
        for variant in &self.variants {
            // Make sure the algorithms used on the variant actually exist on the family (this is basically a check for typos).
            for algorithm_name in variant.flash_algorithms.iter() {
                if !self
                    .flash_algorithms
                    .iter()
                    .any(|algorithm| &algorithm.name == algorithm_name)
                {
                    return Err(format!(
                        "unknown flash algorithm `{}` for variant `{}`",
                        algorithm_name, variant.name
                    ));
                }
            }

            // Check that there is at least one core.
            if let Some(core) = variant.cores.get(0) {
                // Make sure that the core types (architectures) are not mixed.
                let architecture = core.core_type.architecture();
                if variant
                    .cores
                    .iter()
                    .any(|core| core.core_type.architecture() != architecture)
                {
                    return Err(format!(
                        "definition for variant `{}` contains mixed core architectures",
                        variant.name
                    ));
                }
            } else {
                return Err(format!(
                    "definition for variant `{}` does not contain any cores",
                    variant.name
                ));
            }
        }

        Ok(())
    }
}

impl ChipFamily {
    /// Get the different [Chip]s which are part of this
    /// family.
    pub fn variants(&self) -> &[Chip] {
        &self.variants
    }

    /// Get all flash algorithms for this family of chips.
    pub fn algorithms(&self) -> &[RawFlashAlgorithm] {
        &self.flash_algorithms
    }

    /// Try to find a [RawFlashAlgorithm] with a given name.
    pub fn get_algorithm(&self, name: impl AsRef<str>) -> Option<&RawFlashAlgorithm> {
        let name = name.as_ref();
        self.flash_algorithms.iter().find(|elem| elem.name == name)
    }
}
