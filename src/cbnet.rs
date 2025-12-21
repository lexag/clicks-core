use common::protocol::{message::Message, request::ControlAction};
use crossbeam_channel::{unbounded, Receiver, Sender};

#[derive(Clone)]
pub struct CrossbeamNetwork {
    cmd_tx: Sender<ControlAction>,
    pub cmd_rx: Receiver<ControlAction>,
    notif_tx: Sender<Message>,
    pub notif_rx: Receiver<Message>,
}

impl CrossbeamNetwork {
    pub fn new() -> Self {
        let (cmd_tx, cmd_rx): (Sender<ControlAction>, Receiver<ControlAction>) = unbounded();
        let (notif_tx, notif_rx): (Sender<Message>, Receiver<Message>) = unbounded();
        Self {
            cmd_tx,
            cmd_rx,
            notif_tx,
            notif_rx,
        }
    }

    pub fn notify(&self, notif: Message) {
        let _ = self.notif_tx.try_send(notif);
    }

    pub fn command(&self, cmd: ControlAction) {
        let _ = self.cmd_tx.try_send(cmd);
    }
}
