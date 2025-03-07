//! Register types and the core interface for armv6-M

use super::{Dfsr, State, ARM_REGISTER_FILE};

use crate::architecture::arm::sequences::ArmDebugSequence;
use crate::core::{RegisterDescription, RegisterFile, RegisterKind};
use crate::error::Error;
use crate::memory::Memory;
use crate::{
    Architecture, CoreInformation, CoreInterface, CoreRegister, CoreRegisterAddress, CoreStatus,
    CoreType, DebugProbeError, HaltReason, InstructionSet, MemoryInterface,
};
use anyhow::Result;
use bitfield::bitfield;
use std::sync::Arc;
use std::{
    mem::size_of,
    time::{Duration, Instant},
};

bitfield! {
    /// Debug Halting Control and Status Register, DHCSR (see armv6-M Architecture Reference Manual C1.6.3)
    ///
    /// To write this register successfully, you need to set the debug key via [`Dhcsr::enable_write`] first!
    #[derive(Copy, Clone)]
    pub struct Dhcsr(u32);
    impl Debug;
    /// Indicates whether the processor has been reset since the last read of DHCSR:
    ///
    /// `0`: No reset since last DHCSR read.
    /// `1`: At least one reset since last DHCSR read.
    ///
    /// This is a sticky bit, that clears to `0` on a read of DHCSR.
    pub s_reset_st, _: 25;
    /// When not in Debug state, indicates whether the processor has completed
    /// the execution of an instruction since the last read of DHCSR:
    ///
    /// `0`: No instruction has completed since last DHCSR read.
    /// `1`: At least one instructions has completed since last DHCSR read.
    ///
    /// This is a sticky bit, that clears to `0` on a read of DHCSR.
    ///
    /// This bit is UNKNOWN:
    ///
    /// - after a Local reset, but is set to `1` as soon as the processor completes
    /// execution of an instruction.
    /// - when S_LOCKUP is set to 1.
    /// - when S_HALT is set to 1.
    ///
    /// When the processor is not in Debug state, a debugger can check this bit to
    /// determine if the processor is stalled on a load, store or fetch access.
    pub s_retire_st, _: 24;
    /// Indicates whether the processor is locked up because of an unrecoverable exception:
    ///
    /// `0`: Not locked up.\
    /// `1`: Locked up.
    ///
    /// See Unrecoverable exception cases on page B1-206 for more information.
    ///
    /// This bit can only read as `1` when accessed by a remote debugger using the
    /// DAP. The value of `1` indicates that the processor is running but locked up.
    ///
    /// The bit clears to `0` when the processor enters Debug state.
    pub s_lockup, _: 19;
    /// Indicates whether the processor is sleeping:
    ///
    /// `0`: Not sleeping.\
    /// `1`: Sleeping.
    ///
    /// The debugger must set the DHCSR.C_HALT bit to `1` to gain control, or
    /// wait for an interrupt or other wakeup event to wakeup the system
    pub s_sleep, _: 18;
    /// Indicates whether the processor is in Debug state:
    ///
    /// `0`: Not in Debug state.\
    /// `1`: In Debug state.
    pub s_halt, _: 17;
    /// A handshake flag for transfers through the DCRDR:
    ///
    /// - Writing to DCRSR clears the bit to `0`.
    /// - Completion of the DCRDR transfer then sets the bit to `1`.
    ///
    /// For more information about DCRDR transfers see Debug Core Register
    /// Data Register, DCRDR on page C1-292.
    ///
    /// `0`: There has been a write to the DCRDR, but the transfer is
    /// not complete.\
    /// `1`: The transfer to or from the DCRDR is complete.
    ///
    /// This bit is only valid when the processor is in Debug state, otherwise the
    /// bit is UNKNOWN.
    pub s_regrdy, _: 16;
    /// When debug is enabled, the debugger can write to this bit to mask
    /// PendSV, SysTick and external configurable interrupts:
    ///
    /// `0`: Do not mask.\
    /// `1`: Mask PendSV, SysTick and external configurable interrupts.
    ///
    /// The effect of any attempt to change the value of this bit is UNPREDICTABLE
    /// unless both:
    ///
    /// - before the write to DHCSR, the value of the C_HALT bit is 1.
    /// - the write to the DHCSR that changes the C_MASKINTS bit also
    /// writes `1` to the C_HALT bit.
    ///
    /// This means that a single write to DHCSR cannot set the C_HALT to `0` and
    /// change the value of the C_MASKINTS bit.
    ///
    /// The bit does not affect NMI. When DHCSR.C_DEBUGEN is set to `0`, the
    /// value of this bit is UNKNOWN.
    ///
    /// For more information about the use of this bit see Table C1-9 on page C1-282.
    ///
    /// This bit is UNKNOWN after a power-on reset.
    pub c_maskints, set_c_maskints: 3;
    /// Processor step bit. The effects of writes to this bit are:
    ///
    /// `0`: Single-stepping disabled.\
    /// `1`: Single-stepping enabled.
    ///
    /// For more information about the use of this bit see Table C1-9 on
    /// page C1-282.
    ///
    /// This bit is UNKNOWN after a power-on reset.
    pub c_step, set_c_step: 2;
    /// Processor halt bit. The effects of writes to this bit are:
    ///
    /// `0`: Request a halted processor to run.\
    /// `1`: Request a running processor to halt.
    ///
    /// Table C1-9 on page C1-282 shows the effect of writes to this bit when the
    /// processor is in Debug state.
    ///
    /// This bit is `0` after a System reset.
    pub c_halt, set_c_halt: 1;
    ///  Halting debug enable bit:
    ///
    /// `0`: Halting debug disabled.\
    /// `1`: Halting debug enabled.
    ///
    /// If a debugger writes to DHCSR to change the value of this bit from `0` to
    /// 1, it must also write `0` to the C_MASKINTS bit, otherwise behavior is UNPREDICTABLE.
    ///
    /// This bit can only be written from the DAP. Access to the DHCSR from
    /// software running on the processor is IMPLEMENTATION DEFINED.
    /// However, writes to this bit from software running on the processor are ignored.
    ///
    /// This bit is `0` after a power-on reset.
    pub c_debugen, set_c_debugen: 0;
}

impl Dhcsr {
    /// This function sets the bit to enable writes to this register.
    pub fn enable_write(&mut self) {
        self.0 &= !(0xffff << 16);
        self.0 |= 0xa05f << 16;
    }
}

impl From<u32> for Dhcsr {
    fn from(value: u32) -> Self {
        Self(value)
    }
}

impl From<Dhcsr> for u32 {
    fn from(value: Dhcsr) -> Self {
        value.0
    }
}

impl CoreRegister for Dhcsr {
    const ADDRESS: u32 = 0xE000_EDF0;
    const NAME: &'static str = "DHCSR";
}

/// Debug Core Register Data Register, DCRDR (see armv6-M Architecture Reference Manual C1.6.5)
#[derive(Debug, Copy, Clone)]
pub struct Dcrdr(u32);

impl From<u32> for Dcrdr {
    fn from(value: u32) -> Self {
        Self(value)
    }
}

impl From<Dcrdr> for u32 {
    fn from(value: Dcrdr) -> Self {
        value.0
    }
}

impl CoreRegister for Dcrdr {
    const ADDRESS: u32 = 0xE000_EDF8;
    const NAME: &'static str = "DCRDR";
}

bitfield! {
    /// Breakpoint Control register, BP_CTRL (see armv6-M Architecture Reference Manual  C1.8.2)
    ///
    /// Provides BPU implementation information, and the global enable for the BPU.
    #[derive(Copy, Clone)]
    pub struct BpCtrl(u32);
    impl Debug;
    /// The number of breakpoint comparators. If NUM_CODE is zero, the implementation does not support any comparators
    pub num_code, _: 7, 4;
    /// RAZ on reads, SBO, for writes. If written as zero, the write to the register is ignored.
    pub key, set_key: 1;
    /// Enables the BPU:
    ///
    /// `0`: BPU is disabled.\
    /// `1`: BPU is enabled.
    ///
    /// This bit is set to `0` on a power-on reset
    pub _, set_enable: 0;
}

impl From<u32> for BpCtrl {
    fn from(value: u32) -> Self {
        Self(value)
    }
}

impl From<BpCtrl> for u32 {
    fn from(value: BpCtrl) -> Self {
        value.0
    }
}

impl CoreRegister for BpCtrl {
    const ADDRESS: u32 = 0xE000_2000;
    const NAME: &'static str = "BP_CTRL";
}

bitfield! {
    /// Breakpoint Comparator registers, BP_COMPx (see armv6-M Architecture Reference Manual  C1.8.2)
    ///
    /// Holds a breakpoint address for comparison with instruction addresses in the Code memory
    /// region, see The system address map on page B3-224 for more information.
    #[derive(Copy, Clone)]
    pub struct BpCompx(u32);
    impl Debug;
    /// BP_MATCH defines the behavior when the COMP address is matched:
    ///
    /// - `00` no breakpoint matching.
    /// - `01` breakpoint on lower halfword, upper is unaffected.
    /// - `10` breakpoint on upper halfword, lower is unaffected.
    /// - `11` breakpoint on both lower and upper halfwords.
    /// - The field is UNKNOWN on reset.
    pub bp_match, set_bp_match: 31,30;
    /// Stores bits [28:2] of the comparison address. The comparison address is
    /// compared with the address from the Code memory region. Bits [31:29] and
    /// [1:0] of the comparison address are zero.
    ///
    /// The field is UNKNOWN on power-on reset.
    pub comp, set_comp: 28,2;
    /// Enables the comparator:
    ///
    /// `0`: comparator is disabled.\
    /// `1`: comparator is enabled.
    ///
    /// This bit is set to `0` on a power-on reset.
    pub enable, set_enable: 0;
}

impl From<u32> for BpCompx {
    fn from(value: u32) -> Self {
        Self(value)
    }
}

impl From<BpCompx> for u32 {
    fn from(value: BpCompx) -> Self {
        value.0
    }
}

impl CoreRegister for BpCompx {
    const ADDRESS: u32 = 0xE000_2008;
    const NAME: &'static str = "BP_CTRL0";
}

impl BpCompx {
    /// Get the correct comparator value stored at the given address.
    ///
    /// This will adjust the [`BpCompx::comp`] result based on the `BpCompx.bp_match()` specification
    ///
    /// NOTE: Does not support a bp_match value of `11`
    fn get_breakpoint_comparator(register_value: u32) -> Result<u32, Error> {
        let bp_val = BpCompx::from(register_value);
        if bp_val.bp_match() == 0b01 {
            Ok(bp_val.comp() << 2)
        } else if bp_val.bp_match() == 0b10 {
            Ok((bp_val.comp() << 2) | 0x2)
        } else {
            return Err(Error::ArchitectureSpecific(Box::new(DebugProbeError::Other(anyhow::anyhow!("Unsupported breakpoint comparator value {:#08x} for HW breakpoint. Breakpoint must be on half-word boundaries", bp_val.0)))));
        }
    }
}

bitfield! {
    /// Application Interrupt and Reset Control Register, AIRCR (see armv6-M Architecture Reference Manual B3.2.6)
    ///
    /// [`Aircr::vectkey`] must be called before this register can effectively be written!
    #[derive(Copy, Clone)]
    pub struct Aircr(u32);
    impl Debug;
    /// Vector Key. The value 0x05FA must be written to this register, otherwise
    /// the register write is UNPREDICTABLE.
    get_vectkeystat, set_vectkey: 31,16;
    /// Indicates the memory system data endianness:
    ///
    /// `0`: little endian.\
    /// `1`: big endian.
    ///
    /// See Endian support on page A3-44 for more information.
    pub endianness, set_endianness: 15;
    ///  System Reset Request:
    ///
    /// `0`: do not request a reset.\
    /// `1`: request reset.
    ///
    /// Writing `1` to this bit asserts a signal to request a reset by the external
    /// system. The system components that are reset by this request are
    /// IMPLEMENTATION DEFINED. A Local reset is required as part of a system
    /// reset request.
    ///
    /// A Local reset clears this bit to `0`.
    ///
    /// See Reset management on page B1-208 for more information
    pub sysresetreq, set_sysresetreq: 2;
    /// Clears all active state information for fixed and configurable exceptions:
    ///
    /// `0`: do not clear state information.\
    /// `1`: clear state information.
    ///
    /// The effect of writing a `1` to this bit if the processor is not halted in Debug
    /// state is UNPREDICTABLE.
    pub vectclractive, set_vectclractive: 1;
}

impl From<u32> for Aircr {
    fn from(value: u32) -> Self {
        Self(value)
    }
}

impl From<Aircr> for u32 {
    fn from(value: Aircr) -> Self {
        value.0
    }
}

impl Aircr {
    /// Must be called before writing the register.
    pub fn vectkey(&mut self) {
        self.set_vectkey(0x05FA);
    }

    /// Verifies that the vector key is correct (see [`Aircr::vectkey`])
    pub fn vectkeystat(&self) -> bool {
        self.get_vectkeystat() == 0xFA05
    }
}

impl CoreRegister for Aircr {
    const ADDRESS: u32 = 0xE000_ED0C;
    const NAME: &'static str = "AIRCR";
}

bitfield! {
    /// Debug Exception and Monitor Control Register, DEMCR (see armv6-M Architecture Reference Manual C1.6.6)
    #[derive(Copy, Clone)]
    pub struct Demcr(u32);
    impl Debug;
    /// Global enable for DWT.
    ///
    /// Enables:
    /// - Data Watchpoint and Trace (DWT)
    /// - Instrumentation Trace Macrocell (ITM)
    /// - Embedded Trace Macrocell (ETM)
    /// - Trace Port Interface Unit (TPIU).
    pub dwtena, set_dwtena: 24;
    /// Enable halting debug trap on a HardFault exception
    pub vc_harderr, set_vc_harderr: 10;
    /// Enable Reset Vector Catch
    pub vc_corereset, set_vc_corereset: 0;
}

impl From<u32> for Demcr {
    fn from(value: u32) -> Self {
        Self(value)
    }
}

impl From<Demcr> for u32 {
    fn from(value: Demcr) -> Self {
        value.0
    }
}

impl CoreRegister for Demcr {
    const ADDRESS: u32 = 0xe000_edfc;
    const NAME: &'static str = "DEMCR";
}

/*
const REGISTERS: RegisterFile = RegisterFile {
    registers:
    R0: CoreRegisterAddress(0b0_0000),
    R1: CoreRegisterAddress(0b0_0001),
    R2: CoreRegisterAddress(0b0_0010),
    R3: CoreRegisterAddress(0b0_0011),
    R4: CoreRegisterAddress(0b0_0100),
    R5: CoreRegisterAddress(0b0_0101),
    R6: CoreRegisterAddress(0b0_0110),
    R7: CoreRegisterAddress(0b0_0111),
    R8: CoreRegisterAddress(0b0_1000),
    R9: CoreRegisterAddress(0b0_1001),
    PC: CoreRegisterAddress(0b0_1111),
    SP: CoreRegisterAddress(0b0_1101),
    LR: CoreRegisterAddress(0b0_1110),
    XPSR: CoreRegisterAddress(0b1_0000),
};
*/

/// The Main Stack Pointer
pub const MSP: CoreRegisterAddress = CoreRegisterAddress(0b01001);
/// The Process Stack Pointer ([only used with OSes](See ARMv6-M architecture manual B1.4.1 (The SP registers))
pub const PSP: CoreRegisterAddress = CoreRegisterAddress(0b01010);

const PC: RegisterDescription = RegisterDescription {
    name: "PC",
    _kind: RegisterKind::PC,
    address: CoreRegisterAddress(0b0_1111),
};

const XPSR: RegisterDescription = RegisterDescription {
    name: "XPSR",
    _kind: RegisterKind::General,
    address: CoreRegisterAddress(0b1_0000),
};

/// The state of a core that can be used to persist core state across calls to multiple different cores.
pub(crate) struct Armv6m<'probe> {
    memory: Memory<'probe>,

    state: &'probe mut State,

    sequence: Arc<dyn ArmDebugSequence>,
}

impl<'probe> Armv6m<'probe> {
    pub(crate) fn new(
        mut memory: Memory<'probe>,
        state: &'probe mut State,
        sequence: Arc<dyn ArmDebugSequence>,
    ) -> Result<Self, Error> {
        if !state.initialized() {
            // determine current state
            let dhcsr = Dhcsr(memory.read_word_32(Dhcsr::ADDRESS)?);

            let core_state = if dhcsr.s_sleep() {
                CoreStatus::Sleeping
            } else if dhcsr.s_halt() {
                let dfsr = Dfsr(memory.read_word_32(Dfsr::ADDRESS)?);

                let reason = dfsr.halt_reason();

                log::debug!("Core was halted when connecting, reason: {:?}", reason);

                CoreStatus::Halted(reason)
            } else {
                CoreStatus::Running
            };

            // Clear DFSR register. The bits in the register are sticky,
            // so we clear them here to ensure that that none are set.
            let dfsr_clear = Dfsr::clear_all();

            memory.write_word_32(Dfsr::ADDRESS, dfsr_clear.into())?;

            state.current_state = core_state;
            state.initialize();
        }

        Ok(Self {
            memory,
            state,
            sequence,
        })
    }
}

impl<'probe> CoreInterface for Armv6m<'probe> {
    fn wait_for_core_halted(&mut self, timeout: Duration) -> Result<(), Error> {
        // Wait until halted state is active again.
        let start = Instant::now();

        while start.elapsed() < timeout {
            let dhcsr_val = Dhcsr(self.memory.read_word_32(Dhcsr::ADDRESS)?);

            if dhcsr_val.s_halt() {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        Err(Error::Probe(DebugProbeError::Timeout))
    }

    fn core_halted(&mut self) -> Result<bool, Error> {
        // Wait until halted state is active again.
        let dhcsr_val = Dhcsr(self.memory.read_word_32(Dhcsr::ADDRESS)?);

        if dhcsr_val.s_halt() {
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn halt(&mut self, timeout: Duration) -> Result<CoreInformation, Error> {
        // TODO: Generic halt support

        let mut value = Dhcsr(0);
        value.set_c_halt(true);
        value.set_c_debugen(true);
        value.enable_write();

        self.memory.write_word_32(Dhcsr::ADDRESS, value.into())?;

        self.wait_for_core_halted(timeout)?;

        // Update core status
        let _ = self.status()?;

        // try to read the program counter
        let pc_value = self.read_core_reg(PC.address)?;

        // get pc
        Ok(CoreInformation { pc: pc_value })
    }

    fn run(&mut self) -> Result<(), Error> {
        // Before we run, we always perform a single instruction step, to account for possible breakpoints that might get us stuck on the current instruction.
        self.step()?;

        let mut value = Dhcsr(0);
        value.set_c_halt(false);
        value.set_c_debugen(true);
        value.enable_write();

        self.memory.write_word_32(Dhcsr::ADDRESS, value.into())?;
        self.memory.flush()?;

        // We assume that the core is running now.
        self.state.current_state = CoreStatus::Running;

        Ok(())
    }

    fn step(&mut self) -> Result<CoreInformation, Error> {
        // First check if we stopped on a breakpoint, because this requires special handling before we can continue.
        let was_breakpoint =
            if self.state.current_state == CoreStatus::Halted(HaltReason::Breakpoint) {
                log::debug!("Core was halted on breakpoint, disabling breakpoints");
                self.enable_breakpoints(false)?;
                true
            } else {
                false
            };

        let mut value = Dhcsr(0);
        // Leave halted state.
        // Step one instruction.
        value.set_c_step(true);
        value.set_c_halt(false);
        value.set_c_debugen(true);
        value.set_c_maskints(true);
        value.enable_write();

        self.memory.write_word_32(Dhcsr::ADDRESS, value.into())?;
        self.memory.flush()?;
        self.wait_for_core_halted(Duration::from_millis(100))?;

        // Re-enable breakpoints before we continue.
        if was_breakpoint {
            self.enable_breakpoints(true)?;
        }
        // try to read the program counter
        let pc_value = self.read_core_reg(PC.address)?;

        // get pc
        Ok(CoreInformation { pc: pc_value })
    }

    fn reset(&mut self) -> Result<(), Error> {
        self.sequence
            .reset_system(&mut self.memory, crate::CoreType::Armv6m, None)
    }

    fn reset_and_halt(&mut self, _timeout: Duration) -> Result<CoreInformation, Error> {
        self.sequence
            .reset_catch_set(&mut self.memory, crate::CoreType::Armv6m, None)?;
        self.sequence
            .reset_system(&mut self.memory, crate::CoreType::Armv6m, None)?;

        // Update core status
        let _ = self.status()?;

        const XPSR_THUMB: u32 = 1 << 24;
        let xpsr_value = self.read_core_reg(XPSR.address)?;
        if xpsr_value & XPSR_THUMB == 0 {
            self.write_core_reg(XPSR.address, xpsr_value | XPSR_THUMB)?;
        }

        self.sequence
            .reset_catch_clear(&mut self.memory, crate::CoreType::Armv6m, None)?;

        // try to read the program counter
        let pc_value = self.read_core_reg(PC.address)?;

        // get pc
        Ok(CoreInformation { pc: pc_value })
    }

    fn available_breakpoint_units(&mut self) -> Result<u32, Error> {
        let result = self.memory.read_word_32(BpCtrl::ADDRESS)?;

        let register = BpCtrl::from(result);

        Ok(register.num_code())
    }

    fn enable_breakpoints(&mut self, state: bool) -> Result<(), Error> {
        log::debug!("Enabling breakpoints: {:?}", state);
        let mut value = BpCtrl(0);
        value.set_key(true);
        value.set_enable(state);

        self.memory.write_word_32(BpCtrl::ADDRESS, value.into())?;
        self.memory.flush()?;

        self.state.hw_breakpoints_enabled = state;

        Ok(())
    }

    fn set_hw_breakpoint(&mut self, bp_register_index: usize, addr: u32) -> Result<(), Error> {
        log::debug!("Setting breakpoint on address 0x{:08x}", addr);

        // The highest 3 bits of the address have to be zero, otherwise the breakpoint cannot
        // be set at the address.
        if addr >= 0x2000_0000 {
            return Err(Error::ArchitectureSpecific(Box::new(DebugProbeError::Other(anyhow::anyhow!("Unsupported address {:#08x} for HW breakpoint. Breakpoint must be at address < 0x2000_0000.", addr)))));
        }

        let mut value = BpCompx(0);
        if addr % 4 < 2 {
            // match lower halfword
            value.set_bp_match(0b01);
        } else {
            // match higher halfword
            value.set_bp_match(0b10);
        }
        value.set_comp((addr >> 2) & 0x07FF_FFFF);
        value.set_enable(true);

        let register_addr = BpCompx::ADDRESS + (bp_register_index * size_of::<u32>()) as u32;

        self.memory.write_word_32(register_addr, value.into())?;

        Ok(())
    }

    fn registers(&self) -> &'static RegisterFile {
        &ARM_REGISTER_FILE
    }

    fn clear_hw_breakpoint(&mut self, bp_unit_index: usize) -> Result<(), Error> {
        let register_addr = BpCompx::ADDRESS + (bp_unit_index * size_of::<u32>()) as u32;

        let mut value = BpCompx::from(0);
        value.set_enable(false);

        self.memory.write_word_32(register_addr, value.into())?;

        Ok(())
    }

    fn hw_breakpoints_enabled(&self) -> bool {
        self.state.hw_breakpoints_enabled
    }

    fn architecture(&self) -> Architecture {
        Architecture::Arm
    }

    fn core_type(&self) -> CoreType {
        CoreType::Armv6m
    }

    fn instruction_set(&mut self) -> Result<InstructionSet, Error> {
        Ok(InstructionSet::Thumb2)
    }

    fn status(&mut self) -> Result<crate::core::CoreStatus, Error> {
        let dhcsr = Dhcsr(self.memory.read_word_32(Dhcsr::ADDRESS)?);

        if dhcsr.s_lockup() {
            log::warn!("The core is in locked up status as a result of an unrecoverable exception");

            self.state.current_state = CoreStatus::LockedUp;

            return Ok(CoreStatus::LockedUp);
        }

        if dhcsr.s_sleep() {
            // Check if we assumed the core to be halted
            if self.state.current_state.is_halted() {
                log::warn!("Expected core to be halted, but core is running");
            }

            self.state.current_state = CoreStatus::Sleeping;

            return Ok(CoreStatus::Sleeping);
        }

        // TODO: Handle lockup

        if dhcsr.s_halt() {
            let dfsr = Dfsr(self.memory.read_word_32(Dfsr::ADDRESS)?);

            let reason = dfsr.halt_reason();

            // Clear bits from Dfsr register
            self.memory
                .write_word_32(Dfsr::ADDRESS, Dfsr::clear_all().into())?;

            // If the core was halted before, we cannot read the halt reason from the chip,
            // because we clear it directly after reading.
            if self.state.current_state.is_halted() {
                // There shouldn't be any bits set, otherwise it means
                // that the reason for the halt has changed. No bits set
                // means that we have an unkown HaltReason.
                if reason == HaltReason::Unknown {
                    log::debug!("Cached halt reason: {:?}", self.state.current_state);
                    return Ok(self.state.current_state);
                }

                log::debug!(
                    "Reason for halt has changed, old reason was {:?}, new reason is {:?}",
                    &self.state.current_state,
                    &reason
                );
            }

            self.state.current_state = CoreStatus::Halted(reason);

            return Ok(CoreStatus::Halted(reason));
        }

        // Core is neither halted nor sleeping, so we assume it is running.
        if self.state.current_state.is_halted() {
            log::warn!("Core is running, but we expected it to be halted");
        }

        self.state.current_state = CoreStatus::Running;

        Ok(CoreStatus::Running)
    }

    fn read_core_reg(&mut self, address: CoreRegisterAddress) -> Result<u32, Error> {
        self.memory.read_core_reg(address)
    }

    fn write_core_reg(&mut self, address: CoreRegisterAddress, value: u32) -> Result<()> {
        self.memory.write_core_reg(address, value)?;
        Ok(())
    }

    /// See docs on the [`CoreInterface::hw_breakpoints`] trait
    fn hw_breakpoints(&mut self) -> Result<Vec<Option<u32>>, Error> {
        let mut breakpoints = vec![];
        let num_hw_breakpoints = self.available_breakpoint_units()? as usize;
        for bp_unit_index in 0..num_hw_breakpoints {
            let reg_addr = BpCompx::ADDRESS + (bp_unit_index * size_of::<u32>()) as u32;
            // The raw breakpoint address as read from memory
            let register_value = self.memory.read_word_32(reg_addr)?;
            if BpCompx::from(register_value).enable() {
                let breakpoint = BpCompx::get_breakpoint_comparator(register_value)?;
                breakpoints.push(Some(breakpoint));
            } else {
                breakpoints.push(None);
            }
        }
        Ok(breakpoints)
    }
}

impl<'probe> MemoryInterface for Armv6m<'probe> {
    fn read_word_32(&mut self, address: u32) -> Result<u32, Error> {
        self.memory.read_word_32(address)
    }
    fn read_word_8(&mut self, address: u32) -> Result<u8, Error> {
        self.memory.read_word_8(address)
    }
    fn read_32(&mut self, address: u32, data: &mut [u32]) -> Result<(), Error> {
        self.memory.read_32(address, data)
    }
    fn read_8(&mut self, address: u32, data: &mut [u8]) -> Result<(), Error> {
        self.memory.read_8(address, data)
    }
    fn write_word_32(&mut self, address: u32, data: u32) -> Result<(), Error> {
        self.memory.write_word_32(address, data)
    }
    fn write_word_8(&mut self, address: u32, data: u8) -> Result<(), Error> {
        self.memory.write_word_8(address, data)
    }
    fn write_32(&mut self, address: u32, data: &[u32]) -> Result<(), Error> {
        self.memory.write_32(address, data)
    }
    fn write_8(&mut self, address: u32, data: &[u8]) -> Result<(), Error> {
        self.memory.write_8(address, data)
    }
    fn flush(&mut self) -> Result<(), Error> {
        self.memory.flush()
    }
}
