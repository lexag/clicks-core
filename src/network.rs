use std::net::UdpSocket;

use crate::{logger, CrossbeamNetwork};
use chrono::{DateTime, Utc};
use common::{
    command::ControlCommand,
    control::ControlMessage,
    network::{NetworkStatus, SubscriberInfo},
    status::{Notification, NotificationKind},
};
use crossbeam_channel::Sender;
use jack::Control;

pub struct NetworkHandler {
    socket: UdpSocket,
    subscribers: Vec<SubscriberInfo>,
}

impl NetworkHandler {
    pub fn new(port: &str) -> NetworkHandler {
        let nh = NetworkHandler {
            subscribers: vec![],
            socket: UdpSocket::bind(format!("192.168.1.125:{port}"))
                .expect("couldn't open local port"),
        };
        let _ = nh.socket.set_nonblocking(true);
        return nh;
    }

    pub fn start(&mut self) {
        let _ = self.socket.set_nonblocking(true);
    }

    pub fn tick(&mut self) -> Option<ControlMessage> {
        let mut buf = [0; 1024 * 64];
        match self.socket.recv_from(&mut buf) {
            Ok((amt, src)) => {
                for subscriber in &mut self.subscribers {
                    if subscriber.streq(src.to_string()) {
                        subscriber.last_contact = Utc::now().to_rfc3339();
                    }
                }
                let msg: ControlMessage =
                    match serde_json::from_str(std::str::from_utf8(&buf[..amt]).unwrap()) {
                        Ok(msg) => msg,
                        Err(err) => {
                            panic!(
                                "failed parse! {err} \n {}",
                                std::str::from_utf8(&buf[..amt]).unwrap()
                            );
                        }
                    };
                match msg.clone() {
                    ControlMessage::Ping => {}
                    ControlMessage::SubscribeRequest(info) => {
                        let mut recognized_subscriber = false;
                        for subscriber in &mut self.subscribers {
                            if subscriber.address == info.address && subscriber.port == info.port {
                                subscriber.message_kinds = info.message_kinds.clone();
                                recognized_subscriber = true;
                            }
                        }
                        if !recognized_subscriber {
                            logger::log(
                                format!(
                                    "New subscriber: {} at [{}:{}] subscribing to {:?}.",
                                    info.identifier, info.address, info.port, info.message_kinds
                                ),
                                logger::LogContext::Network,
                                logger::LogKind::Note,
                            );
                            self.subscribers.push(info);
                        }
                        self.send_to_all(Notification::NetworkChanged(NetworkStatus {
                            subscribers: self.subscribers.clone(),
                        }));
                        return Some(ControlMessage::NotifySubscribers);
                    }
                    ControlMessage::UnsubscribeRequest(info) => {
                        self.subscribers = self
                            .subscribers
                            .clone()
                            .into_iter()
                            .filter(|sub| !(sub.address == info.address && sub.port == info.port))
                            .collect();
                        self.send_to_all(Notification::NetworkChanged(NetworkStatus {
                            subscribers: self.subscribers.clone(),
                        }));
                        return Some(ControlMessage::NotifySubscribers);
                    }
                    _ => {}
                }
                return Some(msg);
            }
            Err(_err) => {}
        };
        return None;
    }

    pub fn send_to_all(&mut self, msg: Notification) {
        if msg.to_kind() != NotificationKind::TransportChanged {
            logger::log(
                format!("Sending network message: {msg:?}"),
                logger::LogContext::Network,
                logger::LogKind::Debug,
            );
        }
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
            if subscriber.message_kinds.contains(&msg.to_kind()) {
                match self.socket.send_to(
                    serde_json::to_string(&msg).unwrap().as_bytes(),
                    format!("{}:{}", subscriber.address, subscriber.port),
                ) {
                    Ok(amt) => {}
                    Err(err) => {
                        logger::log(
                            format!("Subscriber send error: {err}"),
                            logger::LogContext::Network,
                            logger::LogKind::Error,
                        );
                    }
                }
            }
        }
    }
}
