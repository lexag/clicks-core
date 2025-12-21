use common::{
    local::config::{LogContext, LogKind},
    mem::time::format_hms,
};
use std::{io::Write, path::PathBuf, str::FromStr};

pub fn get_path() -> PathBuf {
    const LOG_PATH_STR: &str = "logs";
    PathBuf::from_str(LOG_PATH_STR).expect("Log path is constant.")
}

pub fn init() {
    let log_path = get_path();
    if !std::fs::exists(&log_path).expect("Log path is always valid") {
        let _ = std::fs::create_dir(&log_path);
    }
    let log_size_total = std::fs::read_dir(&log_path)
        .expect("Log path is always valid.")
        .map(|f| {
            f.expect("File cannot fail here")
                .metadata()
                .expect("Cannot reasonably fail")
                .len() as usize
        })
        .sum::<usize>();

    if log_size_total > 1024 * 1024 * 1024 {
        std::fs::read_dir(&log_path)
            .expect("Log path is always valid")
            .for_each(|rf| {
                let f = rf.expect("Cannot reasonably fail");
                let _ = f
                    .metadata()
                    .expect("Cannot reasonably fail")
                    .modified()
                    .expect("Cannot reasonably fail on target platforms")
                    .elapsed()
                    .unwrap_or_default()
                    .as_secs()
                    > 3600 * 24 * 7;
            });
    }

    let mut time = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let mut time_hash = String::new();
    while time > 0 {
        time_hash
            .push(char::from_digit((time & 0x1F) as u32, 32).expect("moduloed to fit base 32"));
        time >>= 5;
    }

    let _ = std::fs::rename(
        "log.txt",
        log_path.join(
            PathBuf::from_str(&format!("log_{time_hash}.txt"))
                .expect("(Semi-)constant path, cannot fail"),
        ),
    );
    let _ = std::fs::write("log.txt", []);
    log(
        format!("Log start. Saved logs size: {} bytes", log_size_total),
        LogContext::Logger,
        LogKind::Note,
    );
}

pub fn log(msg: String, _context: LogContext, kind: LogKind) {
    let systime = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let hms_time = format_hms(systime);
    let systime_str = hms_time.str();

    let mut file = match std::fs::OpenOptions::new().append(true).open("log.txt") {
        Ok(val) => val,
        Err(_err) => return,
    };

    let mut log_line = format!("[{}] {}: {}\n", systime_str, kind, msg);
    log_line = log_line.trim().to_string();
    log_line.push('\n');
    print!("{}", log_line);
    let _ = file.write(log_line.as_bytes());
}
