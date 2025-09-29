use crate::logger;
use common::config::SystemConfiguration;
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
    logger::log(
        err.to_string(),
        logger::LogContext::Boot,
        logger::LogKind::Error,
    );
}

pub fn find_update_path() -> Result<PathBuf, BootError> {
    find_file_path("clicks.update")
}
pub fn find_show_path() -> Result<PathBuf, BootError> {
    find_file_path("clicks.show")
}

pub fn find_file_path(file_name: &str) -> Result<PathBuf, BootError> {
    let data_path = match std::process::Command::new("find")
        .arg("/")
        .arg("-name")
        .arg(file_name)
        .output()
    {
        Err(err) => {
            return Err(BootError::FileFindFailure(format!("{err}")));
        }
        Ok(res) => {
            logger::log(
                format!(
                    "Found {} at {}",
                    file_name,
                    res.stdout.iter().map(|&c| c as char).collect::<String>()
                ),
                logger::LogContext::Boot,
                logger::LogKind::Note,
            );
            let results = res.stdout.iter().map(|&c| c as char).collect::<String>();
            let path = results.split('\n').nth(0).unwrap_or_default().trim();

            if path.len() == 0 {
                return Err(BootError::FileDoesNotExist);
            } else {
                return Ok(PathBuf::from_str(path).expect("PathBuf cannot fail from_str"));
            }
        }
    };
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

    match serde_json::from_str::<SystemConfiguration>(file_string) {
        Ok(config) => Ok(config),
        Err(err) => Err(BootError::BootProgramOrderFailure(err.to_string())),
    }
}

pub fn write_default_config() -> Result<(), BootError> {
    std::fs::create_dir_all(
        get_config_path()
            .parent()
            .expect("get_config_path() is constant and has a definite parent."),
    );
    std::fs::write(
        get_config_path(),
        serde_json::to_string_pretty(&SystemConfiguration::default()).expect(
            "SystemConfiguration::default() has trivial derived conversion and will never fail.",
        ),
    );
    Ok(())
}

pub fn write_config(config: SystemConfiguration) -> Result<(), BootError> {
    logger::log(
        format!("Saving configuration file...",),
        logger::LogContext::Boot,
        logger::LogKind::Note,
    );

    let config_str = match serde_json::to_string_pretty(&config) {
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

pub fn try_patch() -> Result<bool, ()> {
    let update_path = match find_update_path() {
        Ok(val) => val,
        Err(err) => return Ok(false),
    };

    if let Ok(mut child) = std::process::Command::new("mv")
        .arg(update_path)
        .arg(match std::env::current_exe() {
            Ok(path) => path,
            Err(err) => return Err(()),
        })
        .spawn()
    {
        child.wait();
        return Ok(true);
    }
    return Ok(false);
}
