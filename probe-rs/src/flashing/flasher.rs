use probe_rs_target::{MemoryRegion, RawFlashAlgorithm};

use super::{
    FlashAlgorithm, FlashBuilder, FlashError, FlashFill, FlashLayout, FlashPage, FlashProgress,
};
use crate::config::NvmRegion;
use crate::memory::MemoryInterface;
use crate::{
    core::{Architecture, RegisterFile},
    session::Session,
    Core, CoreRegisterAddress, InstructionSet,
};
use std::{fmt::Debug, time::Duration};

pub(super) trait Operation {
    fn operation() -> u32;
    fn operation_name() -> &'static str {
        match Self::operation() {
            1 => "Erase",
            2 => "Program",
            3 => "Verify",
            _ => "Unknown Operation",
        }
    }
}

pub(super) struct Erase;

impl Operation for Erase {
    fn operation() -> u32 {
        1
    }
}

pub(super) struct Program;

impl Operation for Program {
    fn operation() -> u32 {
        2
    }
}

pub(super) struct Verify;

impl Operation for Verify {
    fn operation() -> u32 {
        3
    }
}

/// A structure to control the flash of an attached microchip.
///
/// Once constructed it can be used to program date to the flash.
pub(super) struct Flasher<'session> {
    session: &'session mut Session,
    core_index: usize,
    flash_algorithm: FlashAlgorithm,
}

impl<'session> Flasher<'session> {
    pub(super) fn new(
        session: &'session mut Session,
        core_index: usize,
        raw_flash_algorithm: &RawFlashAlgorithm,
    ) -> Result<Self, FlashError> {
        let target = session.target();

        // Find a RAM region from which we can run the algo.
        let mm = &target.memory_map;
        let core_name = &target.cores[core_index].name;
        let ram = mm
            .iter()
            .filter_map(|mm| match mm {
                MemoryRegion::Ram(ram) => Some(ram),
                _ => None,
            })
            .find(|ram| {
                // The RAM must be accessible from the core we're going to run the algo on.
                ram.cores.contains(core_name)
            })
            .ok_or(FlashError::NoRamDefined {
                name: session.target().name.clone(),
            })?;

        log::info!("chosen RAM to run the algo: {:x?}", ram);

        let flash_algorithm = FlashAlgorithm::assemble_from_raw(raw_flash_algorithm, ram, target)?;

        let mut this = Self {
            session,
            core_index,
            flash_algorithm,
        };

        this.load()?;

        Ok(this)
    }

    pub(super) fn flash_algorithm(&self) -> &FlashAlgorithm {
        &self.flash_algorithm
    }

    pub(super) fn double_buffering_supported(&self) -> bool {
        self.flash_algorithm.page_buffers.len() > 1
    }

    fn load(&mut self) -> Result<(), FlashError> {
        log::debug!("Initializing the flash algorithm.");
        let algo = &mut self.flash_algorithm;

        // Attach to memory and core.
        let mut core = self
            .session
            .core(self.core_index)
            .map_err(FlashError::Core)?;

        // TODO: Halt & reset target.
        log::debug!("Halting core {}", self.core_index);
        let cpu_info = core
            .halt(Duration::from_millis(100))
            .map_err(FlashError::Core)?;
        log::debug!("PC = 0x{:08x}", cpu_info.pc);
        log::debug!("Reset and halt");
        core.reset_and_halt(Duration::from_millis(500))
            .map_err(FlashError::Core)?;

        // TODO: Possible special preparation of the target such as enabling faster clocks for the flash e.g.

        // Load flash algorithm code into target RAM.
        log::debug!(
            "Loading algorithm into RAM at address 0x{:08x}",
            algo.load_address
        );

        core.write_32(algo.load_address, algo.instructions.as_slice())
            .map_err(FlashError::Core)?;

        let mut data = vec![0; algo.instructions.len()];
        core.read_32(algo.load_address, &mut data)
            .map_err(FlashError::Core)?;

        for (offset, (original, read_back)) in algo.instructions.iter().zip(data.iter()).enumerate()
        {
            if original != read_back {
                log::error!(
                    "Failed to verify flash algorithm. Data mismatch at address {:#08x}",
                    algo.load_address + (4 * offset) as u32
                );
                log::error!("Original instruction: {:#08x}", original);
                log::error!("Readback instruction: {:#08x}", read_back);

                log::error!("Original: {:x?}", &algo.instructions);
                log::error!("Readback: {:x?}", &data);

                return Err(FlashError::FlashAlgorithmNotLoaded);
            }
        }

        log::debug!("RAM contents match flashing algo blob.");

        Ok(())
    }

    pub(super) fn init<O: Operation>(
        &mut self,
        clock: Option<u32>,
    ) -> Result<ActiveFlasher<'_, O>, FlashError> {
        // Attach to memory and core.
        let core = self
            .session
            .core(self.core_index)
            .map_err(FlashError::Core)?;

        log::debug!("Preparing Flasher for operation {}", O::operation_name());
        let mut flasher = ActiveFlasher::<O> {
            core,
            flash_algorithm: self.flash_algorithm.clone(),
            _operation: core::marker::PhantomData,
        };

        flasher.init(clock)?;

        Ok(flasher)
    }

    pub(super) fn run_erase<T, F>(&mut self, f: F) -> Result<T, FlashError>
    where
        F: FnOnce(&mut ActiveFlasher<'_, Erase>) -> Result<T, FlashError> + Sized,
    {
        // TODO: Fix those values (None, None).
        let mut active = self.init(None)?;
        let r = f(&mut active)?;
        active.uninit()?;
        Ok(r)
    }

    pub(super) fn run_program<T, F>(&mut self, f: F) -> Result<T, FlashError>
    where
        F: FnOnce(&mut ActiveFlasher<'_, Program>) -> Result<T, FlashError> + Sized,
    {
        // TODO: Fix those values (None, None).
        let mut active = self.init(None)?;
        let r = f(&mut active)?;
        active.uninit()?;
        Ok(r)
    }

    pub(super) fn run_verify<T, F>(&mut self, f: F) -> Result<T, FlashError>
    where
        F: FnOnce(&mut ActiveFlasher<'_, Verify>) -> Result<T, FlashError> + Sized,
    {
        // TODO: Fix those values (None, None).
        let mut active = self.init(None)?;
        let r = f(&mut active)?;
        active.uninit()?;
        Ok(r)
    }

    pub(super) fn is_chip_erase_supported(&self) -> bool {
        self.flash_algorithm().pc_erase_all.is_some()
    }

    /// Program the contents of given `FlashBuilder` to the flash.
    ///
    /// If `restore_unwritten_bytes` is `true`, all bytes of a sector,
    /// that are not to be written during flashing will be read from the flash first
    /// and written again once the sector is erased.
    pub(super) fn program(
        &mut self,
        region: &NvmRegion,
        flash_builder: &FlashBuilder,
        restore_unwritten_bytes: bool,
        enable_double_buffering: bool,
        skip_erasing: bool,
        progress: &FlashProgress,
    ) -> Result<(), FlashError> {
        log::debug!("Starting program procedure.");
        // Convert the list of flash operations into flash sectors and pages.
        let mut flash_layout = flash_builder.build_sectors_and_pages(
            region,
            &self.flash_algorithm,
            restore_unwritten_bytes,
        )?;

        progress.initialized(flash_layout.clone());

        log::debug!("Double Buffering enabled: {:?}", enable_double_buffering);
        log::debug!(
            "Restoring unwritten bytes enabled: {:?}",
            restore_unwritten_bytes
        );

        // Read all fill areas from the flash.
        progress.started_filling();

        if restore_unwritten_bytes {
            let fills = flash_layout.fills().to_vec();
            for fill in fills {
                let t = std::time::Instant::now();
                let page = &mut flash_layout.pages_mut()[fill.page_index()];
                let result = self.fill_page(page, &fill);

                // If we encounter an error, catch it, gracefully report the failure and return the error.
                if result.is_err() {
                    progress.failed_filling();
                    return result;
                } else {
                    progress.page_filled(fill.size(), t.elapsed());
                }
            }
        }

        // We successfully finished filling.
        progress.finished_filling();

        // Skip erase if necessary
        if !skip_erasing {
            // Erase all necessary sectors
            self.sector_erase(&flash_layout, progress)?;
        }

        // Flash all necessary pages.
        if self.double_buffering_supported() && enable_double_buffering {
            self.program_double_buffer(&flash_layout, progress)?;
        } else {
            self.program_simple(&flash_layout, progress)?;
        };

        Ok(())
    }

    /// Fills all the bytes of `current_page`.
    ///
    /// If `restore_unwritten_bytes` is `true`, all bytes of the page,
    /// that are not to be written during flashing will be read from the flash first
    /// and written again once the page is programmed.
    pub(super) fn fill_page(
        &mut self,
        page: &mut FlashPage,
        fill: &FlashFill,
    ) -> Result<(), FlashError> {
        let page_offset = (fill.address() - page.address()) as usize;
        let page_slice = &mut page.data_mut()[page_offset..page_offset + fill.size() as usize];
        self.run_verify(|active| {
            active
                .core
                .read(fill.address(), page_slice)
                .map_err(FlashError::Core)
        })
    }

    /// Programs the pages given in `flash_layout` into the flash.
    fn program_simple(
        &mut self,
        flash_layout: &FlashLayout,
        progress: &FlashProgress,
    ) -> Result<(), FlashError> {
        progress.started_programming();

        let mut t = std::time::Instant::now();
        let result = self.run_program(|active| {
            for page in flash_layout.pages() {
                active
                    .program_page(page.address(), page.data())
                    .map_err(|error| FlashError::PageWrite {
                        page_address: page.address(),
                        source: Box::new(error),
                    })?;
                progress.page_programmed(page.size(), t.elapsed());
                t = std::time::Instant::now();
            }
            Ok(())
        });

        if result.is_ok() {
            progress.finished_programming();
        } else {
            progress.failed_programming();
        }

        result
    }

    /// Perform an erase of all sectors given in `flash_layout`.
    fn sector_erase(
        &mut self,
        flash_layout: &FlashLayout,
        progress: &FlashProgress,
    ) -> Result<(), FlashError> {
        progress.started_erasing();

        let mut t = std::time::Instant::now();
        let result = self.run_erase(|active| {
            for sector in flash_layout.sectors() {
                active
                    .erase_sector(sector.address())
                    .map_err(|e| FlashError::EraseFailed {
                        sector_address: sector.address(),
                        source: Box::new(e),
                    })?;

                progress.sector_erased(sector.size(), t.elapsed());
                t = std::time::Instant::now();
            }
            Ok(())
        });

        if result.is_ok() {
            progress.finished_erasing();
        } else {
            progress.failed_erasing();
        }

        result
    }

    /// Flash a program using double buffering.
    ///
    /// This uses two buffers to increase the flash speed.
    /// While the data from one buffer is programmed, the
    /// data for the next page is already downloaded
    /// into the next buffer.
    ///
    /// This is only possible if the RAM is large enough to
    /// fit at least two page buffers. See [Flasher::double_buffering_supported].
    fn program_double_buffer(
        &mut self,
        flash_layout: &FlashLayout,
        progress: &FlashProgress,
    ) -> Result<(), FlashError> {
        let mut current_buf = 0;

        progress.started_programming();

        let mut t = std::time::Instant::now();
        let result = self.run_program(|active| {
            let mut last_page_address = 0;
            for page in flash_layout.pages() {
                // At the start of each loop cycle load the next page buffer into RAM.
                active.load_page_buffer(page.address(), page.data(), current_buf)?;

                // Then wait for the active RAM -> Flash copy process to finish.
                // Also check if it finished properly. If it didn't, return an error.
                let result =
                    active
                        .wait_for_completion(Duration::from_secs(2))
                        .map_err(|error| FlashError::PageWrite {
                            page_address: last_page_address,
                            source: Box::new(error),
                        })?;

                last_page_address = page.address();
                progress.page_programmed(page.size(), t.elapsed());
                t = std::time::Instant::now();
                if result != 0 {
                    return Err(FlashError::RoutineCallFailed {
                        name: "program_page",
                        error_code: result,
                    });
                }

                // Start the next copy process.
                active.start_program_page_with_buffer(page.address(), current_buf)?;

                // Swap the buffers
                if current_buf == 1 {
                    current_buf = 0;
                } else {
                    current_buf = 1;
                }
            }

            let result = active
                .wait_for_completion(Duration::from_secs(2))
                .map_err(|error| FlashError::PageWrite {
                    page_address: last_page_address,
                    source: Box::new(error),
                })?;

            if result != 0 {
                Err(FlashError::RoutineCallFailed {
                    name: "wait_for_completion",
                    error_code: result,
                })
            } else {
                Ok(0)
            }
        });

        if result.is_ok() {
            progress.finished_programming();
        } else {
            progress.failed_programming();
            result?;
        }

        Ok(())
    }
}

struct Registers {
    pc: u32,
    r0: Option<u32>,
    r1: Option<u32>,
    r2: Option<u32>,
    r3: Option<u32>,
}

impl Debug for Registers {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{:08x}({:?}, {:?}, {:?}, {:?}",
            self.pc, self.r0, self.r1, self.r2, self.r3
        )
    }
}

pub(super) struct ActiveFlasher<'probe, O: Operation> {
    core: Core<'probe>,
    flash_algorithm: FlashAlgorithm,
    _operation: core::marker::PhantomData<O>,
}

impl<'probe, O: Operation> ActiveFlasher<'probe, O> {
    pub(super) fn init(&mut self, clock: Option<u32>) -> Result<(), FlashError> {
        let algo = &self.flash_algorithm;
        log::debug!("Running init routine.");

        let address = self.flash_algorithm.flash_properties.address_range.start;

        // Execute init routine if one is present.
        if let Some(pc_init) = algo.pc_init {
            let result = self
                .call_function_and_wait(
                    &Registers {
                        pc: pc_init,
                        r0: Some(address),
                        r1: clock.or(Some(0)),
                        r2: Some(O::operation()),
                        r3: None,
                    },
                    true,
                    Duration::from_secs(2),
                )
                .map_err(|error| FlashError::Init(Box::new(error)))?;

            if result != 0 {
                return Err(FlashError::RoutineCallFailed {
                    name: "init",
                    error_code: result,
                });
            }
        }

        Ok(())
    }

    // pub(super) fn session_mut(&mut self) -> &mut Session {
    //     &mut self.session
    // }

    pub(super) fn uninit(&mut self) -> Result<(), FlashError> {
        log::debug!("Running uninit routine.");
        let algo = &self.flash_algorithm;

        if let Some(pc_uninit) = algo.pc_uninit {
            let result = self
                .call_function_and_wait(
                    &Registers {
                        pc: pc_uninit,
                        r0: Some(O::operation()),
                        r1: None,
                        r2: None,
                        r3: None,
                    },
                    false,
                    Duration::from_secs(2),
                )
                .map_err(|error| FlashError::Uninit(Box::new(error)))?;

            if result != 0 {
                return Err(FlashError::RoutineCallFailed {
                    name: "uninit",
                    error_code: result,
                });
            }
        }
        Ok(())
    }

    fn call_function_and_wait(
        &mut self,
        registers: &Registers,
        init: bool,
        duration: Duration,
    ) -> Result<u32, crate::Error> {
        self.call_function(registers, init)?;
        self.wait_for_completion(duration)
    }

    fn call_function(&mut self, registers: &Registers, init: bool) -> Result<(), crate::Error> {
        log::debug!("Calling routine {:?}, init={})", &registers, init);

        let algo = &self.flash_algorithm;
        let regs: &'static RegisterFile = self.core.registers();

        let registers = [
            (regs.program_counter(), Some(registers.pc)),
            (regs.argument_register(0), registers.r0),
            (regs.argument_register(1), registers.r1),
            (regs.argument_register(2), registers.r2),
            (regs.argument_register(3), registers.r3),
            (
                regs.platform_register(9),
                if init { Some(algo.static_base) } else { None },
            ),
            (
                regs.stack_pointer(),
                if init { Some(algo.begin_stack) } else { None },
            ),
            (
                regs.return_address(),
                // For ARM Cortex-M cores, we have to add 1 to the return address,
                // to ensure that we stay in Thumb mode.
                if self.core.instruction_set()? == InstructionSet::Thumb2 {
                    Some(algo.load_address + 1)
                } else {
                    Some(algo.load_address)
                },
            ),
        ];

        for (description, value) in &registers {
            if let Some(v) = value {
                self.core.write_core_reg(description.address, *v)?;
                log::debug!(
                    "content of {} {:#x}: 0x{:08x} should be: 0x{:08x}",
                    description.name,
                    description.address.0,
                    self.core.read_core_reg(description.address)?,
                    *v
                );
            }
        }

        if self.core.architecture() == Architecture::Riscv {
            // Ensure ebreak enters debug mode, this is necessary for soft breakpoints to work.
            let dcsr = self.core.read_core_reg(CoreRegisterAddress::from(0x7b0))?;

            self.core.write_core_reg(
                CoreRegisterAddress::from(0x7b0),
                dcsr | (1 << 15) | (1 << 13) | (1 << 12),
            )?;
        }

        // Resume target operation.
        self.core.run()?;

        Ok(())
    }

    pub(super) fn wait_for_completion(&mut self, timeout: Duration) -> Result<u32, crate::Error> {
        log::debug!("Waiting for routine call completion.");
        let regs = self.core.registers();

        self.core.wait_for_core_halted(timeout)?;

        let r = self.core.read_core_reg(regs.result_register(0).address)?;
        Ok(r)
    }
}

impl<'probe> ActiveFlasher<'probe, Erase> {
    pub(super) fn erase_all(&mut self) -> Result<(), FlashError> {
        log::debug!("Erasing entire chip.");
        let flasher = self;
        let algo = &flasher.flash_algorithm;

        if let Some(pc_erase_all) = algo.pc_erase_all {
            let result = flasher
                .call_function_and_wait(
                    &Registers {
                        pc: pc_erase_all,
                        r0: None,
                        r1: None,
                        r2: None,
                        r3: None,
                    },
                    false,
                    Duration::from_secs(30),
                )
                .map_err(|error| FlashError::ChipEraseFailed {
                    source: Box::new(error),
                })?;

            if result != 0 {
                Err(FlashError::ChipEraseFailed {
                    source: Box::new(FlashError::RoutineCallFailed {
                        name: "chip_erase",
                        error_code: result,
                    }),
                })
            } else {
                Ok(())
            }
        } else {
            Err(FlashError::ChipEraseNotSupported)
        }
    }

    pub(super) fn erase_sector(&mut self, address: u32) -> Result<(), FlashError> {
        log::info!("Erasing sector at address 0x{:08x}", address);
        let t1 = std::time::Instant::now();

        let result = self
            .call_function_and_wait(
                &Registers {
                    pc: self.flash_algorithm.pc_erase_sector,
                    r0: Some(address),
                    r1: None,
                    r2: None,
                    r3: None,
                },
                false,
                Duration::from_millis(
                    self.flash_algorithm.flash_properties.erase_sector_timeout as u64,
                ),
            )
            .map_err(|error| FlashError::EraseFailed {
                sector_address: address,
                source: Box::new(error),
            })?;
        log::info!(
            "Done erasing sector. Result is {}. This took {:?}",
            result,
            t1.elapsed()
        );

        if result != 0 {
            Err(FlashError::RoutineCallFailed {
                name: "erase_sector",
                error_code: result,
            })
        } else {
            Ok(())
        }
    }
}

impl<'p> ActiveFlasher<'p, Program> {
    pub(super) fn program_page(&mut self, address: u32, bytes: &[u8]) -> Result<(), FlashError> {
        let t1 = std::time::Instant::now();

        log::info!(
            "Flashing page at address {:#08x} with size: {}",
            address,
            bytes.len()
        );

        // Transfer the bytes to RAM.
        self.core
            .write_8(self.flash_algorithm.begin_data, bytes)
            .map_err(FlashError::Core)?;

        let result = self
            .call_function_and_wait(
                &Registers {
                    pc: self.flash_algorithm.pc_program_page,
                    r0: Some(address),
                    r1: Some(bytes.len() as u32),
                    r2: Some(self.flash_algorithm.begin_data),
                    r3: None,
                },
                false,
                Duration::from_millis(
                    self.flash_algorithm.flash_properties.program_page_timeout as u64,
                ),
            )
            .map_err(|error| FlashError::PageWrite {
                page_address: address,
                source: Box::new(error),
            })?;
        log::info!("Flashing took: {:?}", t1.elapsed());

        if result != 0 {
            Err(FlashError::PageWrite {
                page_address: address,
                source: Box::new(FlashError::RoutineCallFailed {
                    name: "program_page",
                    error_code: result,
                }),
            })
        } else {
            Ok(())
        }
    }

    pub(super) fn start_program_page_with_buffer(
        &mut self,
        address: u32,
        buffer_number: usize,
    ) -> Result<(), FlashError> {
        // Ensure the buffer number is valid, otherwise there is a bug somewhere
        // in the flashing code.
        assert!(
            buffer_number < self.flash_algorithm.page_buffers.len(),
            "Trying to use non-existing buffer ({}/{}) for flashing. This is a bug. Please report it.",
            buffer_number, self.flash_algorithm.page_buffers.len()
        );

        self.call_function(
            &Registers {
                pc: self.flash_algorithm.pc_program_page,
                r0: Some(address),
                r1: Some(self.flash_algorithm.flash_properties.page_size),
                r2: Some(self.flash_algorithm.page_buffers[buffer_number as usize]),
                r3: None,
            },
            false,
        )
        .map_err(|error| FlashError::PageWrite {
            page_address: address,
            source: Box::new(error),
        })?;

        Ok(())
    }

    pub(super) fn load_page_buffer(
        &mut self,
        _address: u32,
        bytes: &[u8],
        buffer_number: usize,
    ) -> Result<(), FlashError> {
        let algo = &self.flash_algorithm;

        // Ensure the buffer number is valid, otherwise there is a bug somewhere
        // in the flashing code.
        assert!(
            buffer_number < algo.page_buffers.len(),
            "Trying to use non-existing buffer ({}/{}) for flashing. This is a bug. Please report it.",
            buffer_number, algo.page_buffers.len()
        );

        // TODO: Prevent security settings from locking the device.
        // Transfer the buffer bytes to RAM.
        let words: Vec<u32> = bytes
            .chunks_exact(core::mem::size_of::<u32>())
            .map(|a| u32::from_le_bytes([a[0], a[1], a[2], a[3]]))
            .collect();

        let t1 = std::time::Instant::now();
        self.core
            .write_32(algo.page_buffers[buffer_number], &words)
            .map_err(FlashError::Core)?;

        log::info!(
            "Took {:?} to download {} byte page into ram",
            t1.elapsed(),
            bytes.len()
        );

        Ok(())
    }
}
