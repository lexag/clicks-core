use std::{
    net::{IpAddr, SocketAddr},
    str::FromStr,
};

use crate::{
    communication::{interface::CommunicationInterface, netport::NetworkPort},
    logger,
};
use chrono::{DateTime, Utc};
use common::{
    control::ControlMessage,
    network::{NetworkStatus, SubscriberInfo},
    status::{Notification, NotificationKind},
};

pub struct JsonNetHandler {
    port: NetworkPort,
    subscribers: Vec<SubscriberInfo>,
    input_queue: Vec<ControlMessage>,
}

impl JsonNetHandler {
    pub fn new(port: usize) -> Self {
        let a = Self {
            port: NetworkPort::new(port),
            subscribers: vec![],
            input_queue: vec![],
        };
        logger::log(
            format!(
                "opened jsonnet port {}",
                a.port.socket.local_addr().unwrap()
            ),
            common::config::LogContext::Network,
            common::config::LogKind::Note,
        );
        a
    }
}
impl CommunicationInterface for JsonNetHandler {
    fn get_inputs(&mut self, limit: usize) -> Vec<ControlMessage> {
        let mut inputs: Vec<ControlMessage> = vec![];
        inputs.append(&mut self.input_queue);
        while let Some((buf, amt, src)) = self.port.recv() {
            for subscriber in &mut self.subscribers {
                if subscriber.streq(src.to_string()) {
                    subscriber.last_contact = Utc::now().to_rfc3339();
                }
            }
            let msg: ControlMessage =
                match serde_json::from_str(match std::str::from_utf8(&buf[..amt]) {
                    Ok(val) => val,
                    Err(err) => panic!("failed conversion! {err}",),
                }) {
                    Ok(msg) => msg,
                    Err(err) => {
                        panic!(
                            "failed parse! {err} \n {}",
                            std::str::from_utf8(&buf[..amt]).unwrap_or_default()
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
                    self.notify(Notification::NetworkChanged(NetworkStatus {
                        subscribers: self.subscribers.clone(),
                    }));
                    self.input_queue.push(ControlMessage::NotifySubscribers);
                }
                ControlMessage::UnsubscribeRequest(info) => {
                    self.subscribers = self
                        .subscribers
                        .clone()
                        .into_iter()
                        .filter(|sub| !(sub.address == info.address && sub.port == info.port))
                        .collect();
                    self.notify(Notification::NetworkChanged(NetworkStatus {
                        subscribers: self.subscribers.clone(),
                    }));
                }
                _ => {}
            }
            self.input_queue.push(msg);
            if inputs.len() + self.input_queue.len() > limit {
                break;
            } else {
                inputs.append(&mut self.input_queue);
            }
        }
        return inputs;
    }

    fn notify_multiple(&mut self, notifications: Vec<Notification>) {
        for notif in notifications {
            self.notify(notif);
        }
    }

    fn notify(&mut self, notification: Notification) {
        if false && notification.to_kind() != NotificationKind::TransportChanged {
            logger::log(
                format!("Sending network message: {notification:?}"),
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
                    .signed_duration_since(
                        DateTime::parse_from_rfc3339(&sub.last_contact).unwrap_or_default(),
                    )
                    .num_minutes()
                    < 15
            })
            .collect();

        for subscriber in &self.subscribers {
            if subscriber.message_kinds.contains(&notification.to_kind()) {
                self.port.send_to(
                    serde_json::to_string(&notification)
                        .expect("notification has trivial derived conversion")
                        .as_bytes(),
                    SocketAddr::new(
                        IpAddr::from_str(&subscriber.address)
                            .expect("all subscriber addresses are santizied earlier"),
                        subscriber
                            .port
                            .parse()
                            .expect("all subscriber ports are sanitized earlier"),
                    ),
                );
            }
        }
    }
}
