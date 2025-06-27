use std::{
    fmt::{Debug, Display},
    io::Write,
    path::PathBuf,
    str::FromStr,
};

pub enum LogKind {
    Error,
    Warning,
    Note,
    Command,
    Debug,
}

pub enum LogContext {
    Logger,
    Network,
    AudioProcessor,
    AudioSource,
    AudioHandler,
    Boot,
}

impl Display for LogKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match *self {
            LogKind::Error => write!(f, "ERROR"),
            LogKind::Warning => write!(f, "WARNING"),
            LogKind::Note => write!(f, "NOTE"),
            LogKind::Command => write!(f, "COMMAND"),
            LogKind::Debug => write!(f, "DEBUG"),
        }
    }
}

const LOG_PATH_STR: &str = "logs";

pub fn get_path() -> PathBuf {
    let LOG_PATH = PathBuf::from_str(LOG_PATH_STR).unwrap();
    LOG_PATH
}

pub fn init() {
    let LOG_PATH = PathBuf::from_str(LOG_PATH_STR).unwrap();
    if !std::fs::exists(&LOG_PATH).unwrap() {
        std::fs::create_dir(&LOG_PATH);
    }
    let log_size_total = std::fs::read_dir(&LOG_PATH)
        .unwrap()
        .map(|f| f.unwrap().metadata().unwrap().len() as usize)
        .sum::<usize>();

    if log_size_total > 1024 * 1024 * 1024 {
        std::fs::read_dir(&LOG_PATH).unwrap().for_each(|rf| {
            let f = rf.unwrap();
            f.metadata()
                .unwrap()
                .modified()
                .unwrap()
                .elapsed()
                .unwrap()
                .as_secs()
                > 3600 * 24 * 7;
        });
    }

    let mut time = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let mut time_hash = String::new();
    while time > 0 {
        time_hash.push(char::from_digit((time & 0x1F) as u32, 32).unwrap());
        time >>= 5;
    }

    std::fs::rename(
        "log.txt",
        LOG_PATH.join(PathBuf::from_str(&format!("log_{time_hash}.txt")).unwrap()),
    );
    std::fs::write("log.txt", []);
    log(
        format!("Log start. Saved logs size: {} bytes", log_size_total),
        LogContext::Logger,
        LogKind::Note,
    );
}

pub fn log(msg: String, context: LogContext, kind: LogKind) {
    let LOG_PATH = PathBuf::from_str(LOG_PATH_STR).unwrap();
    let systime = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let systime_str = common::time::format_hms(systime);

    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .append(true)
        .open("log.txt")
        .unwrap();

    let mut log_line = format!("[{}] {}: {}\n", systime_str, kind.to_string(), msg);
    log_line = log_line.trim().to_string();
    log_line.push('\n');
    print!("{}", log_line);
    file.write(log_line.as_bytes());
}
