use rppal::i2c;

use core::fmt::Write;

pub fn display_test() -> Result<(), i2c::Error> {
    let mut interface = i2c::I2c::new()?;
    // Send command: Display ON (0xAF)
    interface.block_write(0x00, &[0xAF])?;
    println!("Sent display ON command");

    // Send test data byte (0xFF)
    interface.block_write(0x40, &[0xFF])?;
    println!("Sent test pixel data");

    Ok(())
}
