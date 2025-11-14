use rppal::i2c::I2c;

bitflags::bitflags! {
    #[derive(Default)]
pub struct HwButton: u8 {
    const YES = 0x01;
    const NO = 0x02;
}}

fn get_buttons() -> Result<HwButton, Box<dyn std::error::Error>> {
    let mut i2c = I2c::new()?;
    i2c.set_slave_address(0x55);
    let mut buf = [0u8; 1];
    i2c.read(&mut buf);
    Ok(HwButton::from_bits(buf[0]).unwrap_or_default())
}

pub fn wait_yes_no() -> bool {
    loop {
        let buttons = get_buttons().unwrap_or_default();
        if buttons.contains(HwButton::NO) {
            return false;
        } else if buttons.contains(HwButton::YES) {
            return true;
        }
    }
}
