use std::net::{Ipv4Addr, UdpSocket};

use chrono::{DateTime, Utc};
use common::{
    command::ControlCommand,
    network::{ControlMessageKind, NetworkStatus, StatusMessageKind, SubscriberInfo},
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
            Ok((amt, src)) => {
                for subscriber in &mut self.subscribers {
                    if subscriber.streq(src.to_string()) {
                        subscriber.last_contact = Utc::now().to_rfc3339();
                    }
                }
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
                    ControlMessageKind::Ping => {}
                    ControlMessageKind::ControlCommand(cmd) => {
                        let _ = self.cmd_tx.send(cmd);
                    }
                    ControlMessageKind::SubscribeRequest(info) => {
                        let mut recognized_subscriber = false;
                        for subscriber in &mut self.subscribers {
                            if subscriber.address == info.address && subscriber.port == info.port {
                                subscriber.message_kinds = info.message_kinds.clone();
                                recognized_subscriber = true;
                            }
                        }
                        if !recognized_subscriber {
                            println!("New subscriber: {info:?}");
                            self.subscribers.push(info);
                        }
                        let _ = self.cmd_tx.send(ControlCommand::NotifySubscribers);
                        self.send_to_all(StatusMessageKind::NetworkStatus(Some(NetworkStatus {
                            subscribers: self.subscribers.clone(),
                        })));
                    }
                    ControlMessageKind::UnsubscribeRequest(info) => {
                        self.subscribers = self
                            .subscribers
                            .clone()
                            .into_iter()
                            .filter(|sub| !(sub.address == info.address && sub.port == info.port))
                            .collect();
                        let _ = self.cmd_tx.send(ControlCommand::NotifySubscribers);
                        self.send_to_all(StatusMessageKind::NetworkStatus(Some(NetworkStatus {
                            subscribers: self.subscribers.clone(),
                        })));
                    }
                    _ => {}
                }
            }
            Err(_err) => {}
        };
    }

    pub fn send_to_all(&mut self, msg: StatusMessageKind) {
        self.subscribers = self
            .subscribers
            .clone()
            .into_iter()
            .filter(|sub| {
                Utc::now()
                    .signed_duration_since(DateTime::parse_from_rfc3339(&sub.last_contact).unwrap())
                    .num_minutes()
                    < 15
            })
            .collect();

        for subscriber in &self.subscribers {
            for msg_kind in &subscriber.message_kinds {
                if std::mem::discriminant(&msg) == std::mem::discriminant(msg_kind) {
                    //println!("{}", serde_json::to_string(&msg).unwrap());
                    match self.socket.send_to(
                        serde_json::to_string(&msg).unwrap().as_bytes(),
                        format!("{}:{}", subscriber.address, subscriber.port),
                    ) {
                        Ok(amt) => {
                            //println!("{}", amt);
                        }
                        Err(err) => {
                            println!("{}", err);
                        }
                    }
                }
            }
        }
    }
}
