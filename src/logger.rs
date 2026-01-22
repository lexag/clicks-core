use crate::{cbnet::CrossbeamNetwork, hardware};
use common::{
    local::config::{LogContext, LogItem, LogKind},
    mem::time::format_hms,
};
use std::{io::Write, path::PathBuf, str::FromStr};

#[derive(Default)]
pub struct LogDispatcher {
    kind_filter: LogKind,
    context_filter: LogContext,
    cbnet: CrossbeamNetwork,
    file_handler: Option<LogFileHandler>,
}

impl LogDispatcher {
    pub fn new(cbnet: CrossbeamNetwork) -> Self {
        let mut log_queue = Vec::<LogItem>::new();

        let file_handler = LogFileHandler::new("logs".into(), 16000000);
        if let Err(ref err) = file_handler {
            log_queue.push(LogItem::new(
                format!("Error occured on LogDispatcher LogFileHandler: {err}"),
                LogContext::Logger,
                LogKind::Error,
            ));
        }

        let a = Self {
            kind_filter: LogKind::all(),
            context_filter: LogContext::all(),
            cbnet,
            file_handler: file_handler.ok(),
        };

        for log in log_queue {
            a.log(log);
        }

        return a;
    }

    pub fn tick(&self) -> Result<(), std::io::Error> {
        while let Ok(item) = self.cbnet.log_rx.try_recv() {
            self.log(item)?;
        }
        Ok(())
    }

    pub fn log(&self, item: LogItem) -> Result<(), std::io::Error> {
        if item.kind.intersects(self.kind_filter) && item.context.intersects(self.context_filter) {
            if let Some(handler) = &self.file_handler {
                handler.log_to_file(&item)?;
            }

            self.cbnet.notify(common::protocol::message::Message::Large(
                common::protocol::message::LargeMessage::Log(item.clone()),
            ));
            // also send message
        }
        if item.kind.intersects(LogKind::Error | LogKind::Warning) {
            hardware::display::generic_failure(item.message)?;
        }
        Ok(())
    }
}

#[derive(Default)]
pub struct LogFileHandler {
    log_path: PathBuf,
    // in bytes
    log_dir_max_size: usize,
}

impl LogFileHandler {
    fn new(path: PathBuf, max_size: usize) -> Result<Self, std::io::Error> {
        let full_path = std::env::current_dir()
            .map_err(|_| std::io::ErrorKind::PermissionDenied)?
            .join(path);
        if !std::fs::exists(&full_path)? {
            let _ = std::fs::create_dir_all(&full_path);
        }

        let a = Self {
            log_path: full_path,
            log_dir_max_size: max_size,
        };

        a.archive_current_log()?;
        a.log_dir_size_check()?;
        a.init_new_log()?;

        Ok(a)
    }

    pub fn log_to_file(&self, item: &LogItem) -> Result<(), std::io::Error> {
        let systime = format_hms(item.time / 1000);
        let hms_time = systime.str();

        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(self.log_path.join("log.txt"))?;

        let log_line = format!("[{}] {}: {}\n", hms_time, item.kind, item.message.trim());
        print!("{}", log_line);
        let _ = file.write(log_line.as_bytes())?;
        Ok(())
    }

    pub fn log_dir_size(&self) -> Result<usize, std::io::Error> {
        let dir_read = std::fs::read_dir(&self.log_path)?;
        let mut size = 0;
        for obj in dir_read {
            size += obj?.metadata()?.len()
        }
        Ok(size as usize)
    }

    fn delete_old_logs(&self, age_limit_days: usize) -> Result<(), std::io::Error> {
        let dir_read = std::fs::read_dir(&self.log_path)?;
        const SECS_PER_DAY: usize = 86400;
        for obj in dir_read {
            let hook = obj?;
            if hook
                .metadata()?
                .modified()?
                .elapsed()
                .map_err(|_| std::io::ErrorKind::TimedOut)?
                .as_secs() as usize
                > age_limit_days * SECS_PER_DAY
            {
                std::fs::remove_file(hook.path())?
            }
        }
        Ok(())
    }

    pub fn log_dir_size_check(&self) -> Result<(), std::io::Error> {
        if self.log_dir_size()? > self.log_dir_max_size {
            self.delete_old_logs(30)?
        }
        Ok(())
    }

    pub fn archive_current_log(&self) -> Result<(), std::io::Error> {
        let mut time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| std::io::ErrorKind::Other)?
            .as_secs();
        let mut time_hash = String::new();
        while time > 0 {
            time_hash.push(char::from_digit((time & 0x1F) as u32, 32).unwrap_or_default());
            time >>= 5;
        }

        let _ = std::fs::rename(
            "log.txt",
            self.log_path.join(
                PathBuf::from_str(&format!("log_{time_hash}.txt"))
                    .expect("(Semi-)constant path, cannot fail"),
            ),
        );
        Ok(())
    }

    pub fn init_new_log(&self) -> Result<(), std::io::Error> {
        std::fs::write(self.log_path.join("log.txt"), [])?;
        Ok(())
    }
}
