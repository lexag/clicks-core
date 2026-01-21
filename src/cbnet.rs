use crate::logger::LogItem;
use common::protocol::{message::Message, request::ControlAction};
use crossbeam_channel::{unbounded, Receiver, Sender};

#[derive(Clone)]
pub struct CrossbeamNetwork {
    cmd_tx: Sender<ControlAction>,
    pub cmd_rx: Receiver<ControlAction>,
    notif_tx: Sender<Message>,
    pub notif_rx: Receiver<Message>,
    log_tx: Sender<LogItem>,
    pub log_rx: Receiver<LogItem>,
}

impl CrossbeamNetwork {
    pub fn new() -> Self {
        let (cmd_tx, cmd_rx): (Sender<ControlAction>, Receiver<ControlAction>) = unbounded();
        let (notif_tx, notif_rx): (Sender<Message>, Receiver<Message>) = unbounded();
        let (log_tx, log_rx): (Sender<LogItem>, Receiver<LogItem>) = unbounded();
        Self {
            cmd_tx,
            cmd_rx,
            notif_tx,
            notif_rx,
            log_tx,
            log_rx,
        }
    }

    pub fn notify(&self, notif: Message) {
        let _ = self.notif_tx.try_send(notif);
    }

    pub fn command(&self, cmd: ControlAction) {
        let _ = self.cmd_tx.try_send(cmd);
    }

    pub fn log(&self, log_item: LogItem) {
        let _ = self.log_tx.try_send(log_item);
    }
}
impl Default for CrossbeamNetwork {
    fn default() -> Self {
        Self::new()
    }
}
