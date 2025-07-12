use crate::logger;
use common::config::{BootProgramOrder, SystemConfiguration};
use std::{fmt::Display, path::PathBuf, str::FromStr};

#[derive(Debug)]
pub enum BootError {
    ShowFindFailure(String),
    ShowDoesNotExist,
    BootProgramOrderFailure(String),
    ConfigWriteError(String),
    LogCopyFailure(String),
}

impl Display for BootError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            BootError::ShowDoesNotExist => {
                write!(f, "Could not find clicks show data. No results. Exiting.")
            }
            BootError::ShowFindFailure(errstr) => write!(
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
        }
    }
}

pub fn log_boot_error(err: BootError) {
    logger::log(
        err.to_string(),
        logger::LogContext::Boot,
        logger::LogKind::Error,
    );
}

pub fn find_show_path() -> Result<PathBuf, BootError> {
    let data_path = match std::process::Command::new("find")
        .arg("/")
        .arg("-name")
        .arg("clicks.show")
        .output()
    {
        Err(err) => {
            return Err(BootError::ShowFindFailure(format!("{err}")));
        }
        Ok(res) => {
            logger::log(
                format!(
                    "Found show data path: {}",
                    res.stdout.iter().map(|&c| c as char).collect::<String>()
                ),
                logger::LogContext::Boot,
                logger::LogKind::Note,
            );
            let results = res.stdout.iter().map(|&c| c as char).collect::<String>();
            let path = results.split('\n').nth(0).unwrap_or_default().trim();

            if path.len() == 0 {
                return Err(BootError::ShowDoesNotExist);
            } else {
                return Ok(PathBuf::from_str(path).unwrap());
            }
        }
    };
}

pub fn get_config_path() -> PathBuf {
    return PathBuf::from_str("~/.config/clicks/clicks.conf")
        .expect("Config file path conversion failed.");
}

pub fn get_config() -> Result<SystemConfiguration, BootError> {
    if !std::fs::exists(get_config_path()).unwrap_or_default() {
        std::fs::create_dir_all(get_config_path().parent().unwrap());
        std::fs::write(
            get_config_path(),
            serde_json::to_string_pretty(&SystemConfiguration::default()).unwrap(),
        );
    }
    match serde_json::from_str::<SystemConfiguration>(
        std::str::from_utf8(&std::fs::read(get_config_path()).unwrap()).unwrap(),
    ) {
        Ok(config) => Ok(config),
        Err(err) => Err(BootError::BootProgramOrderFailure(err.to_string())),
    }
}

pub fn write_default_config(path: PathBuf) -> Result<(), BootError> {
    logger::log(
        format!("Writing new config file and exiting.",),
        logger::LogContext::Boot,
        logger::LogKind::Note,
    );
    match std::fs::write(
        path.join("config.json"),
        serde_json::to_string_pretty(&common::config::SystemConfiguration::default()).unwrap(),
    ) {
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
