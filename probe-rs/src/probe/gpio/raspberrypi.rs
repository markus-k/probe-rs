use super::BitbangInterface;

use std::error::Error;

use rppal::gpio::{Gpio, IoPin, Level, Mode, OutputPin, PullUpDown};

#[derive(Debug)]
pub struct RaspberryPiBitbangInterface {
    clk_pin: OutputPin,
    data_pin: IoPin,
}

impl RaspberryPiBitbangInterface {
    pub fn from_pins(
        clk_pin_no: u8,
        data_pin_no: u8,
    ) -> Result<Self, Box<dyn Error + Send + Sync>> {
        let gpio = Gpio::new()?;

        println!("Clk pin:  {clk_pin_no}");
        println!("Data pin: {data_pin_no}");

        let clk_pin = gpio.get(clk_pin_no)?.into_output();
        let mut data_pin = gpio.get(data_pin_no)?.into_io(Mode::Input);
        data_pin.set_pullupdown(PullUpDown::Off);

        Ok(Self { clk_pin, data_pin })
    }
}

impl BitbangInterface for RaspberryPiBitbangInterface {
    fn read(&self) -> bool {
        match self.data_pin.read() {
            Level::Low => false,
            Level::High => true,
        }
    }

    fn drive(&mut self, out: bool) {
        self.data_pin.set_mode(match out {
            true => Mode::Output,
            false => Mode::Input,
        });
    }

    fn write(&mut self, clk: bool, data: bool) {
        self.data_pin.write(match data {
            true => Level::High,
            false => Level::Low,
        });
        self.clk_pin.write(match clk {
            true => Level::High,
            false => Level::Low,
        });
    }
}
