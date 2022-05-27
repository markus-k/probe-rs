#[cfg(feature = "gpio-raspberrypi")]
pub mod raspberrypi;

use crate::{
    architecture::{
        arm::{
            communication_interface::{DapProbe, UninitializedArmProbe},
            ArmCommunicationInterface, PortType, RawDapAccess, SwoAccess,
        },
        riscv::communication_interface::RiscvCommunicationInterface,
    },
    probe::{jlink::arm::RawProtocolIo, DebugProbe, DebugProbeError},
    DebugProbeInfo, DebugProbeSelector, DebugProbeType, ProbeCreationError, WireProtocol,
};

use std::{fmt::Debug, time::Duration};

use super::{
    jlink::arm::{ProbeStatistics, SwdSettings},
    JTAGAccess,
};

pub trait BitbangInterface: Send + Debug {
    fn read(&self) -> bool;
    fn drive(&mut self, out: bool);
    fn write(&mut self, clk: bool, data: bool);
}

#[derive(Debug)]
struct DummyBitbangInterface {
    clk: bool,
    data: bool,
    is_driven: bool,
}

impl DummyBitbangInterface {
    fn new() -> Self {
        Self {
            clk: false,
            data: true,
            is_driven: true,
        }
    }
}

impl BitbangInterface for DummyBitbangInterface {
    fn read(&self) -> bool {
        log::trace!("reading from gpio: {}", self.data);

        self.data
    }

    fn drive(&mut self, out: bool) {
        log::trace!("setting data drive: {}", out);

        self.is_driven = out;
    }

    fn write(&mut self, clk: bool, data: bool) {
        log::trace!("setting gpio: clk = {}, data = {}", clk, data);

        self.clk = clk;
        self.data = data;
    }
}

#[derive(Debug)]
pub struct GpioDap {
    clk_sleep: Duration,

    bitbanger: Box<dyn BitbangInterface>,

    swd_settings: SwdSettings,
    statistics: ProbeStatistics,
}

impl GpioDap {}

impl DebugProbe for GpioDap {
    fn new_from_selector(
        selector: impl Into<DebugProbeSelector>,
    ) -> Result<Box<Self>, DebugProbeError>
    where
        Self: Sized,
    {
        let selector = selector.into();
        let bbi: Box<dyn BitbangInterface> = if let Some(ref kind) = selector.serial_number {
            let r: Result<Box<dyn BitbangInterface>, DebugProbeError> = match kind.as_str() {
                "dummy" => Ok(Box::new(DummyBitbangInterface::new())),
                #[cfg(feature = "gpio-raspberrypi")]
                "rpi-gpio" => {
                    let clk_pin = selector.vendor_id as u8;
                    let data_pin = selector.product_id as u8;

                    let bbi =
                        raspberrypi::RaspberryPiBitbangInterface::from_pins(clk_pin, data_pin);
                    match bbi {
                        Ok(bbi) => Ok(Box::new(bbi)),
                        Err(bbierr) => Err(DebugProbeError::ProbeCouldNotBeCreated(
                            ProbeCreationError::ProbeSpecific(bbierr),
                        )),
                    }
                }
                _ => Err(DebugProbeError::ProbeCouldNotBeCreated(
                    ProbeCreationError::NotFound,
                )),
            };

            r
        } else {
            Err(DebugProbeError::ProbeCouldNotBeCreated(
                ProbeCreationError::NotFound,
            ))
        }?;

        Ok(Box::new(GpioDap {
            clk_sleep: Duration::from_micros(1), // 500 kHz
            bitbanger: bbi,

            swd_settings: SwdSettings::default(),
            statistics: ProbeStatistics::default(),
        }))
    }

    fn get_name(&self) -> &str {
        "GPIO Debug Probe"
    }

    fn speed_khz(&self) -> u32 {
        1_000_000 / self.clk_sleep.as_nanos() as u32 / 2
    }

    fn set_speed(&mut self, speed_khz: u32) -> Result<u32, DebugProbeError> {
        self.clk_sleep = Duration::from_nanos((1_000_000 / speed_khz / 2).into());

        Ok(self.speed_khz())
    }

    fn attach(&mut self) -> Result<(), DebugProbeError> {
        Ok(())
    }

    fn detach(&mut self) -> Result<(), DebugProbeError> {
        Ok(())
    }

    fn target_reset(&mut self) -> Result<(), DebugProbeError> {
        Err(DebugProbeError::NotImplemented("target_reset"))
    }

    fn target_reset_assert(&mut self) -> Result<(), DebugProbeError> {
        Err(DebugProbeError::NotImplemented("target_reset_assert"))
    }

    fn target_reset_deassert(&mut self) -> Result<(), DebugProbeError> {
        Err(DebugProbeError::NotImplemented("target_reset_deassert"))
    }

    fn select_protocol(&mut self, protocol: WireProtocol) -> Result<(), DebugProbeError> {
        if protocol == WireProtocol::Swd {
            Ok(())
        } else {
            Err(DebugProbeError::UnsupportedProtocol(protocol))
        }
    }

    fn active_protocol(&self) -> Option<WireProtocol> {
        // only supports SWD for now
        Some(WireProtocol::Swd)
    }

    fn has_arm_interface(&self) -> bool {
        true
    }

    fn try_get_arm_interface<'probe>(
        self: Box<Self>,
    ) -> Result<Box<dyn UninitializedArmProbe + 'probe>, (Box<dyn DebugProbe>, DebugProbeError)>
    {
        let unitialized_interface = ArmCommunicationInterface::new(self, true);

        Ok(Box::new(unitialized_interface))
    }

    fn into_probe(self: Box<Self>) -> Box<dyn DebugProbe> {
        self
    }

    fn try_as_dap_probe(&mut self) -> Option<&mut dyn DapProbe> {
        Some(self)
    }

    fn try_get_riscv_interface(
        self: Box<Self>,
    ) -> Result<RiscvCommunicationInterface, (Box<dyn DebugProbe>, DebugProbeError)> {
        Err((
            DebugProbe::into_probe(self),
            DebugProbeError::InterfaceNotAvailable("RISCV"),
        ))
    }

    fn has_riscv_interface(&self) -> bool {
        false
    }

    fn get_swo_interface(&self) -> Option<&dyn SwoAccess> {
        None
    }

    fn get_swo_interface_mut(&mut self) -> Option<&mut dyn SwoAccess> {
        None
    }

    fn get_target_voltage(&mut self) -> Result<Option<f32>, DebugProbeError> {
        Ok(None)
    }
}

impl DapProbe for GpioDap {}

pub(crate) fn list_gpio_devices() -> Vec<DebugProbeInfo> {
    vec![DebugProbeInfo::new(
        format!("gpio0"),
        0,
        0,
        None,
        DebugProbeType::Gpio,
        None,
    )]
}

impl RawProtocolIo for GpioDap {
    fn jtag_io<M, I>(&mut self, _tms: M, _tdi: I) -> Result<Vec<bool>, DebugProbeError>
    where
        M: IntoIterator<Item = bool>,
        I: IntoIterator<Item = bool>,
    {
        todo!()
    }

    fn swd_io<D, S>(&mut self, dir: D, swdio: S) -> Result<Vec<bool>, DebugProbeError>
    where
        D: IntoIterator<Item = bool>,
        S: IntoIterator<Item = bool>,
    {
        let spin_sleeper = spin_sleep::SpinSleeper::new(100_000)
            .with_spin_strategy(spin_sleep::SpinStrategy::SpinLoopHint);
        let zipped = std::iter::zip(dir, swdio);

        let mut result_bits = Vec::new();

        for (is_output, value) in zipped {
            self.bitbanger.drive(is_output);

            self.bitbanger.write(false, value);
            spin_sleeper.sleep(self.clk_sleep);

            result_bits.push(self.bitbanger.read());

            self.bitbanger.write(true, value);
            spin_sleeper.sleep(self.clk_sleep);
        }

        Ok(result_bits)
    }

    fn swd_settings(&self) -> &SwdSettings {
        &self.swd_settings
    }

    fn probe_statistics(&mut self) -> &mut ProbeStatistics {
        &mut self.statistics
    }

    fn line_reset(&mut self) -> Result<(), DebugProbeError> {
        println!("LINE RESET!");

        let mut result = Ok(());

        const NUM_RESET_BITS: u8 = 50;

        for _ in 0..2 {
            self.probe_statistics().report_line_reset();

            self.swj_sequence(NUM_RESET_BITS, 0x7FFFFFFFFFFFF)?;

            let read_result = self.raw_read_register(PortType::DebugPort, 0);
            // Parse the response after the reset bits.
            match read_result {
                Ok(_) => {
                    // Line reset was succesful
                    return Ok(());
                }
                Err(e) => {
                    // Try again, first reset might fail.
                    result = Err(e);
                }
            }
        }

        result
    }
}

impl JTAGAccess for GpioDap {
    fn read_register(&mut self, _address: u32, _len: u32) -> Result<Vec<u8>, DebugProbeError> {
        todo!()
    }

    fn set_idle_cycles(&mut self, _idle_cycles: u8) {}

    fn get_idle_cycles(&self) -> u8 {
        0
    }

    fn set_ir_len(&mut self, _len: u32) {}

    fn write_register(
        &mut self,
        _address: u32,
        _data: &[u8],
        _len: u32,
    ) -> Result<Vec<u8>, DebugProbeError> {
        todo!()
    }
}
