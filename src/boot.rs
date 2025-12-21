use crate::logger;
use common::local::config::{LogContext, LogKind, SystemConfiguration};
use std::{fmt::Display, path::PathBuf, str::FromStr};

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
    PathBuf::from_str("/media/usb_mem/").map_err(|_| BootError::FileDoesNotExist)
}

fn get_pwd() -> Result<PathBuf, BootError> {
    Ok(std::env::current_exe()
        .map_err(|_| BootError::FileDoesNotExist)?
        .parent()
        .ok_or(BootError::FileDoesNotExist)?
        .to_path_buf())
}

pub fn get_config_path() -> PathBuf {
    PathBuf::from_str(".config/clicks/clicks.conf").expect("PathBuf cannot fail from_str")
}

pub fn get_config() -> Result<SystemConfiguration, BootError> {
    if !std::fs::exists(get_config_path()).unwrap_or_default() {
        write_default_config()?;
    }
    let file_content = match std::fs::read(get_config_path()) {
        Ok(content) => content,
        Err(err) => return Err(BootError::FileReadError(err.to_string())),
    };

    let file_string = match std::str::from_utf8(&file_content) {
        Ok(string) => string,
        Err(err) => return Err(BootError::FileReadError(err.to_string())),
    };

    match serde_json::from_str::<SystemConfiguration>(file_string) {
        Ok(config) => Ok(config),
        Err(err) => Err(BootError::BootProgramOrderFailure(err.to_string())),
    }
}

pub fn write_default_config() -> Result<(), BootError> {
    let _ = std::fs::create_dir_all(
        get_config_path()
            .parent()
            .expect("get_config_path() is constant and has a definite parent."),
    );
    let _ = std::fs::write(
        get_config_path(),
        serde_json::to_string(&SystemConfiguration::default()).expect(
            "SystemConfiguration::default() has trivial derived conversion and will never fail.",
        ),
    );
    Ok(())
}

pub fn write_config(config: SystemConfiguration) -> Result<(), BootError> {
    logger::log(
        "Saving configuration file...".to_string(),
        LogContext::Boot,
        LogKind::Note,
    );

    let config_str = match serde_json::to_string(&config) {
        Ok(val) => val,
        Err(err) => return Err(BootError::ConfigWriteError(err.to_string())),
    };

    match std::fs::write(get_config_path(), config_str) {
        Ok(_) => Ok(()),
        Err(err) => Err(BootError::ConfigWriteError(err.to_string())),
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
            Err(_) => return false,
        })
        .arg(match std::env::current_exe() {
            Ok(path) => path,
            Err(_) => return false,
        })
        .spawn()
    {
        let _ = child.wait();
        return true;
    }
    false
}

pub fn try_load_usb_show() -> Result<(), BootError> {
    println!("{:?}, {:?}", get_usb_show_path(), get_show_path());
    if let Ok(mut child) = std::process::Command::new("cp")
        .arg("-r")
        .arg(get_usb_show_path()?)
        .arg(
            get_show_path()?
                .parent()
                .ok_or(BootError::FileDoesNotExist)?,
        )
        .spawn()
    {
        let _ = child.wait();
        return Ok(());
    }
    Err(BootError::FileDoesNotExist)
}
