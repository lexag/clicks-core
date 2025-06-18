use std::net::UdpSocket;

use common::{
    command::ControlCommand,
    network::{ControlMessageKind, StatusMessageKind, SubscriberInfo},
};
use crossbeam_channel::Sender;

pub struct NetworkHandler {
    socket: UdpSocket,
    cmd_tx: Sender<ControlCommand>,
    subscribers: Vec<SubscriberInfo>,
}

impl NetworkHandler {
    pub fn new(port: &str, cmd_tx: Sender<ControlCommand>) -> NetworkHandler {
        let nh = NetworkHandler {
            subscribers: vec![],
            cmd_tx,
            socket: UdpSocket::bind(format!("127.0.0.1:{port}")).expect("couldn't open local port"),
        };
        let _ = nh.socket.set_nonblocking(true);
        return nh;
    }

    pub fn start(&mut self) {
        let _ = self.socket.set_nonblocking(true);
    }

    pub fn tick(&mut self) {
        let mut buf = [0; 1024];
        match self.socket.recv_from(&mut buf) {
            Ok((amt, _src)) => {
                let msg: ControlMessageKind =
                    match serde_json::from_str(std::str::from_utf8(&buf[..amt]).unwrap()) {
                        Ok(msg) => msg,
                        Err(err) => {
                            panic!(
                                "failed parse! {err} \n {}",
                                std::str::from_utf8(&buf[..amt]).unwrap()
                            );
                        }
                    };
                match msg {
                    ControlMessageKind::ControlCommand(cmd) => {
                        let _ = self.cmd_tx.send(cmd);
                    }
                    ControlMessageKind::SubscribeRequest(info) => {
                        println!("New subscriber: {info:?}");
                        self.subscribers.push(info);
                    }
                    _ => {}
                }
            }
            Err(_err) => {}
        };
    }

    pub fn send_to_all(&self, msg: StatusMessageKind) {
        for subscriber in &self.subscribers {
            for msg_kind in &subscriber.message_kinds {
                if std::mem::discriminant(&msg) == std::mem::discriminant(msg_kind) {
                    let _ = self.socket.send_to(
                        serde_json::to_string(&msg).unwrap().as_bytes(),
                        format!("{}:{}", subscriber.address, subscriber.port),
                    );
                }
            }
        }
    }
}
