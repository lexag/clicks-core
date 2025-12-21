use ssd1306::{
    mode::{DisplayConfig, TerminalMode},
    prelude::I2CInterface,
    Ssd1306,
};

use crate::VERSION;
use common::{cue::Show, VERSION as COMMON_VERSION};
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

pub fn patch_success() -> Result<(), Box<dyn std::error::Error>> {
    let mut display = get_display()?;
    ip_header(&mut display)?;
    typewriter(&mut display, "Update succeeded");
    typewriter(&mut display, "");
    typewriter(&mut display, "Please reboot");

    Ok(())
}
pub fn patch_failure() -> Result<(), Box<dyn std::error::Error>> {
    let mut display = get_display()?;
    ip_header(&mut display)?;
    typewriter(&mut display, "");
    typewriter(&mut display, "Update failed");
    typewriter(&mut display, "");
    typewriter(&mut display, "Something");
    typewriter(&mut display, "went wrong");

    Ok(())
}
pub fn show_load_failure(err_str: &str) -> Result<(), Box<dyn std::error::Error>> {
    let mut display = get_display()?;
    ip_header(&mut display)?;
    typewriter(&mut display, "Show load failed");
    typewriter(&mut display, err_str);

    Ok(())
}

pub fn show_load_success(show: &Show) -> Result<(), Box<dyn std::error::Error>> {
    let mut display = get_display()?;
    ip_header(&mut display)?;
    typewriter(&mut display, "Loaded show");
    typewriter(&mut display, show.metadata.name.str());
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
    ip_header(&mut display)?;
    typewriter(&mut display, "port 8081");

    Ok(())
}

pub fn ask_usb() -> Result<(), Box<dyn std::error::Error>> {
    let mut display = get_display()?;
    typewriter(&mut display, "");
    typewriter(&mut display, "Load USB?");
    typewriter(&mut display, "Plug in now");
    typewriter(&mut display, "");
    typewriter(&mut display, "YES/NO");

    Ok(())
}

pub fn ask_patch() -> Result<(), Box<dyn std::error::Error>> {
    let mut display = get_display()?;
    typewriter(&mut display, "");
    typewriter(&mut display, "Update found");
    typewriter(&mut display, "");
    typewriter(&mut display, "Update?");

    Ok(())
}

pub fn ask_copy_show() -> Result<(), Box<dyn std::error::Error>> {
    let mut display = get_display()?;
    typewriter(&mut display, "");
    typewriter(&mut display, "USB show found");
    typewriter(&mut display, "");
    typewriter(&mut display, "Load to core?");

    Ok(())
}

pub fn generic_success() -> Result<(), Box<dyn std::error::Error>> {
    let mut display = get_display()?;
    typewriter(&mut display, "");
    typewriter(&mut display, "Success!");
    typewriter(&mut display, "");

    Ok(())
}
pub fn generic_failure(err: String) -> Result<(), Box<dyn std::error::Error>> {
    let mut display = get_display()?;
    typewriter(&mut display, "Op. failed");
    typewriter(&mut display, &err);

    Ok(())
}

fn typewriter(
    display: &mut Ssd1306<I2CInterface<I2cdev>, DisplaySize128x64, TerminalMode>,
    string: &str,
) {
    for c in string.to_string().chars() {
        let _ = display.print_char(c);
        std::thread::sleep(Duration::from_millis(5));
    }
    let _ = display.print_char('\n');
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

pub fn debug_print(str: String) -> Result<(), Box<dyn std::error::Error>> {
    let mut display = get_display()?;
    typewriter(&mut display, &str);

    Ok(())
}
