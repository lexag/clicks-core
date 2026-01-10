use crate::{cbnet::CrossbeamNetwork, hardware};
use chrono::Utc;
use common::{
    local::config::{LogContext, LogKind},
    mem::time::format_hms,
};
use std::{io::Write, path::PathBuf, str::FromStr};

#[derive(Default, Clone)]
pub struct LogItem {
    message: String,
    kind: LogKind,
    context: LogContext,
    time: u64,
}

impl LogItem {
    pub fn new(msg: String, context: LogContext, kind: LogKind) -> Self {
        Self {
            message: msg,
            kind,
            context,
            time: Utc::now().timestamp_millis() as u64,
        }
    }
}

#[derive(Default)]
pub struct LogDispatcher {
    kind_filter: LogKind,
    context_filter: LogContext,
    cbnet: CrossbeamNetwork,
}

impl LogDispatcher {
    pub fn new(cbnet: CrossbeamNetwork) -> Self {
        Self {
            kind_filter: LogKind::all(),
            context_filter: LogContext::all(),
            cbnet,
        }
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
    pub fn tick(&self) -> Result<(), std::io::Error> {
        while let Ok(item) = self.cbnet.log_rx.try_recv() {
            self.log(item)?;
        }
        Ok(())
    }

    pub fn log(&self, item: LogItem) -> Result<(), std::io::Error> {
        if item.kind.intersects(self.kind_filter) && item.context.intersects(self.context_filter) {
            self.log_to_file(&item)?;
            // also send message
        }
        if item.kind.intersects(LogKind::Error | LogKind::Warning) {
            hardware::display::generic_failure(item.message)?;
        }
        Ok(())
    }

    fn log_to_file(&self, item: &LogItem) -> Result<(), std::io::Error> {
        let systime = format_hms(item.time / 1000);
        let hms_time = systime.str();

        let mut file = std::fs::OpenOptions::new().append(true).open("log.txt")?;

        let log_line = format!("[{}] {}: {}\n", hms_time, item.kind, item.message.trim());
        print!("{}", log_line);
        let _ = file.write(log_line.as_bytes())?;
        Ok(())
    }

    pub fn get_path() -> PathBuf {
        const LOG_PATH_STR: &str = "logs";
        PathBuf::from_str(LOG_PATH_STR).expect("Log path is constant.")
    }
}
