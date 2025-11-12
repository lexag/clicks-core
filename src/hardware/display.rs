use rppal::i2c;
use ssd1306::{
    mode::{DisplayConfig, TerminalMode},
    prelude::I2CInterface,
    Ssd1306,
};

use crate::VERSION;
use common::{show::Show, VERSION as COMMON_VERSION};
use core::fmt::Write;
use linux_embedded_hal::I2cdev;
use local_ip_address::local_ip;
use ssd1306::size::DisplaySize128x64;
use std::{net::IpAddr, str::FromStr, time::Duration};

fn get_display() -> Result<
    Ssd1306<I2CInterface<I2cdev>, DisplaySize128x64, TerminalMode>,
    Box<dyn std::error::Error>,
> {
    let i2cdev = I2cdev::new("/dev/i2c-1")?;

    let interface = ssd1306::I2CDisplayInterface::new(i2cdev);
    let mut display = ssd1306::Ssd1306::new(
        interface,
        DisplaySize128x64,
        ssd1306::prelude::DisplayRotation::Rotate0,
    )
    .into_terminal_mode();
    display.init().unwrap();
    let _ = display.clear();
    Ok(display)
}

pub fn show_load_failure(err: serde_json::Error) -> Result<(), Box<dyn std::error::Error>> {
    let mut display = get_display()?;
    ip_header(&mut display);
    typewriter(&mut display, "Show load fail");
    typewriter(&mut display, &format!("{:?}", err.classify()));
    typewriter(&mut display, &format!("lin {}", err.line()));
    typewriter(&mut display, &format!("col {}", err.column()));

    Ok(())
}

pub fn show_load_success(show: Show) -> Result<(), Box<dyn std::error::Error>> {
    let mut display = get_display()?;
    ip_header(&mut display);
    typewriter(&mut display, "Loaded show");
    typewriter(&mut display, &show.metadata.name);
    typewriter(&mut display, &format!("{} cues", show.cues.len()));

    Ok(())
}

pub fn startup() -> Result<(), Box<dyn std::error::Error>> {
    let mut display = get_display()?;
    typewriter(&mut display, "Karspexet ClicKS");
    typewriter(&mut display, "");
    typewriter(&mut display, &format!("version {}", VERSION));
    typewriter(&mut display, &format!(" common {}", COMMON_VERSION));
    typewriter(&mut display, "");
    ip_header(&mut display);
    typewriter(&mut display, "port 8081");

    Ok(())
}

fn typewriter(
    display: &mut Ssd1306<I2CInterface<I2cdev>, DisplaySize128x64, TerminalMode>,
    string: &str,
) {
    for c in string.to_string().chars() {
        display.print_char(c);
        std::thread::sleep(Duration::from_millis(5));
    }
    display.print_char('\n');
    std::thread::sleep(Duration::from_millis(5));
}

fn ip_header(
    display: &mut Ssd1306<I2CInterface<I2cdev>, DisplaySize128x64, TerminalMode>,
) -> Result<(), Box<dyn std::error::Error>> {
    typewriter(
        display,
        &local_ip()
            .unwrap_or(IpAddr::from_str("0.0.0.0")?)
            .to_string(),
    );
    Ok(())
}
