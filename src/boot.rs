use crate::logger;
use bincode::config::{standard, BigEndian, Configuration, Fixint};
use common::local::config::{LogContext, LogKind, SystemConfiguration};
use std::{default, fmt::Display, path::PathBuf, str::FromStr};

#[derive(Debug)]
pub enum BootError {
    FileFindFailure(String),
    FileDoesNotExist,
    BootProgramOrderFailure(String),
    ConfigWriteError(String),
    LogCopyFailure(String),
    FileReadError(String),
}

impl Display for BootError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            BootError::FileDoesNotExist => {
                write!(f, "Could not find clicks show data. No results. Exiting.")
            }
            BootError::FileFindFailure(errstr) => write!(
                f,
                "Could not find clicks show data. Unknown error: {errstr}"
            ),
            BootError::BootProgramOrderFailure(errstr) => write!(
                f,
                "An error occured when reading boot program order: {errstr}"
            ),
            BootError::ConfigWriteError(errstr) => {
                write!(f, "An error occured when writing configuration: {errstr}")
            }
            BootError::LogCopyFailure(errstr) => {
                write!(f, "An error occured when copying log files: {errstr}")
            }
            BootError::FileReadError(errstr) => {
                write!(f, "Could not read file: {errstr}")
            }
        }
    }
}

pub fn log_boot_error(err: BootError) {
    logger::log(err.to_string(), LogContext::Boot, LogKind::Error);
}

pub fn get_usb_update_path() -> Result<PathBuf, BootError> {
    Ok(get_usb_mountpoint()?.join("clicks.update"))
}
pub fn get_usb_show_path() -> Result<PathBuf, BootError> {
    Ok(get_usb_mountpoint()?.join("clicks.show"))
}
pub fn get_show_path() -> Result<PathBuf, BootError> {
    Ok(get_pwd()?.join("program_memory/clicks.show"))
}

fn get_usb_mountpoint() -> Result<PathBuf, BootError> {
    PathBuf::from_str("/media/usb_mem/").map_err(|e| BootError::FileDoesNotExist)
}

fn get_pwd() -> Result<PathBuf, BootError> {
    Ok(std::env::current_exe()
        .map_err(|e| BootError::FileDoesNotExist)?
        .parent()
        .ok_or(BootError::FileDoesNotExist)?
        .to_path_buf())
}

pub fn get_config_path() -> PathBuf {
    return PathBuf::from_str(".config/clicks/clicks.conf").expect("PathBuf cannot fail from_str");
}

pub fn get_config() -> Result<SystemConfiguration, BootError> {
    if !std::fs::exists(get_config_path()).unwrap_or_default() {
        write_default_config();
    }
    let file_content = match std::fs::read(get_config_path()) {
        Ok(content) => content,
        Err(err) => return Err(BootError::FileReadError(err.to_string())),
    };

    let file_string = match std::str::from_utf8(&file_content) {
        Ok(string) => string,
        Err(err) => return Err(BootError::FileReadError(err.to_string())),
    };

    let config = Configuration::<BigEndian, Fixint>::default()
        .with_big_endian()
        .with_fixed_int_encoding();
    match bincode::decode_from_slice::<SystemConfiguration, Configuration<BigEndian, Fixint>>(
        file_string.as_bytes(),
        config,
    ) {
        Ok(config) => Ok(config.0),
        Err(err) => Err(BootError::BootProgramOrderFailure(err.to_string())),
    }
}

pub fn write_default_config() -> Result<(), BootError> {
    std::fs::create_dir_all(
        get_config_path()
            .parent()
            .expect("get_config_path() is constant and has a definite parent."),
    );
    let bcconfig = Configuration::<BigEndian, Fixint>::default()
        .with_big_endian()
        .with_fixed_int_encoding();
    std::fs::write(
        get_config_path(),
        bincode::encode_to_vec(SystemConfiguration::default(), bcconfig).expect(
            "SystemConfiguration::default() has trivial derived conversion and will never fail.",
        ),
    );
    Ok(())
}

pub fn write_config(config: SystemConfiguration) -> Result<(), BootError> {
    logger::log(
        format!("Saving configuration file...",),
        LogContext::Boot,
        LogKind::Note,
    );

    let bcconfig = Configuration::<BigEndian, Fixint>::default()
        .with_big_endian()
        .with_fixed_int_encoding();
    let config_str = match bincode::encode_to_vec(config, bcconfig) {
        Ok(val) => val,
        Err(err) => return Err(BootError::ConfigWriteError(err.to_string())),
    };

    match std::fs::write(get_config_path(), config_str) {
        Ok(_) => return Ok(()),
        Err(err) => return Err(BootError::ConfigWriteError(err.to_string())),
    }
}

pub fn copy_logs(path: PathBuf) -> Result<(), BootError> {
    match std::fs::copy(logger::get_path(), path.join("logs/")) {
        Ok(_) => Ok(()),
        Err(err) => Err(BootError::LogCopyFailure(err.to_string())),
    }
}

pub fn try_patch() -> bool {
    if let Ok(mut child) = std::process::Command::new("mv")
        .arg(match get_usb_update_path() {
            Ok(path) => path,
            Err(err) => return false,
        })
        .arg(match std::env::current_exe() {
            Ok(path) => path,
            Err(err) => return false,
        })
        .spawn()
    {
        child.wait();
        return true;
    }
    return false;
}

pub fn try_load_usb_show() -> bool {
    println!("{:?}, {:?}", get_usb_show_path(), get_show_path());
    if let Ok(mut child) = std::process::Command::new("cp")
        .arg("-r")
        .arg(match get_usb_show_path() {
            Ok(path) => path,
            Err(err) => return false,
        })
        .arg(match get_show_path() {
            Ok(path) => path,
            Err(err) => return false,
        })
        .spawn()
    {
        child.wait();
        return true;
    }
    return false;
}
