use common::{command::ControlCommand, status::Notification};
use crossbeam_channel::{unbounded, Receiver, Sender};

#[derive(Clone)]
pub struct CrossbeamNetwork {
    cmd_tx: Sender<ControlCommand>,
    pub cmd_rx: Receiver<ControlCommand>,
    notif_tx: Sender<Notification>,
    pub notif_rx: Receiver<Notification>,
}

impl CrossbeamNetwork {
    pub fn new() -> Self {
        let (cmd_tx, cmd_rx): (Sender<ControlCommand>, Receiver<ControlCommand>) = unbounded();
        let (notif_tx, notif_rx): (Sender<Notification>, Receiver<Notification>) = unbounded();
        Self {
            cmd_tx,
            cmd_rx,
            notif_tx,
            notif_rx,
        }
    }

    pub fn notify(&self, notif: Notification) {
        self.notif_tx.try_send(notif);
    }

    pub fn command(&self, cmd: ControlCommand) {
        self.cmd_tx.try_send(cmd);
    }
}
