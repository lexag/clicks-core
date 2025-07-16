use common::{
    command::ControlCommand,
    status::{Notification, ProcessStatus},
};
use crossbeam_channel::{unbounded, Receiver, Sender};

#[derive(Clone)]
pub struct CrossbeamNetwork {
    cmd_tx: Sender<ControlCommand>,
    pub cmd_rx: Receiver<ControlCommand>,
    notif_tx: Sender<Notification>,
    pub notif_rx: Receiver<Notification>,
    status_tx: Sender<ProcessStatus>,
    pub status_rx: Receiver<ProcessStatus>,
}

impl CrossbeamNetwork {
    pub fn new() -> Self {
        let (cmd_tx, cmd_rx): (Sender<ControlCommand>, Receiver<ControlCommand>) = unbounded();
        let (status_tx, status_rx): (Sender<ProcessStatus>, Receiver<ProcessStatus>) = unbounded();
        let (notif_tx, notif_rx): (Sender<Notification>, Receiver<Notification>) = unbounded();
        Self {
            cmd_tx,
            cmd_rx,
            notif_tx,
            notif_rx,
            status_tx,
            status_rx,
        }
    }

    pub fn notify(&self, notif: Notification) {
        self.notif_tx.try_send(notif);
    }

    pub fn send_status(&self, status: ProcessStatus) {
        self.status_tx.try_send(status);
    }

    pub fn command(&self, cmd: ControlCommand) {
        self.cmd_tx.try_send(cmd);
    }
}
