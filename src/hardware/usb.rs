pub fn unmount() {
    if let Ok(mut child) = std::process::Command::new("pumount").arg("usb_mem").spawn() {
        let _ = child.wait();
    }
}

pub fn mount() {
    if let Ok(mut child) = std::process::Command::new("pmount")
        .arg("/dev/sda1")
        .arg("usb_mem")
        .spawn()
    {
        let _ = child.wait();
    }
}
