use std::{
    fmt::Display,
    net::{IpAddr, SocketAddr},
    str::FromStr,
};

use crate::{
    communication::{interface::CommunicationInterface, netport::NetworkPort},
    logger,
};
use chrono::{DateTime, Utc};
use common::{
    local::{
        config::{LogContext, LogKind},
        status::NetworkStatus,
    },
    mem::{
        network::{IpAddress, SubscriberInfo},
        typeflags::MessageType,
    },
    protocol::{message::Message, request::Request},
};
use core::fmt;

pub struct JsonNetHandler {
    port: NetworkPort,
    subscribers: Vec<SubscriberInfo>,
    input_queue: Vec<Request>,
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
            LogContext::Network,
            LogKind::Note,
        );
        a
    }

    pub fn publish_subscribers(&mut self) {
        let subs_slice: [Option<SubscriberInfo>; 32] =
            std::array::from_fn(|i| self.subscribers.get(i).cloned().map(Some).unwrap_or(None));
        self.notify(Message::NetworkChanged(NetworkStatus {
            subscribers: subs_slice,
        }));
    }
}

impl CommunicationInterface for JsonNetHandler {
    fn get_inputs(&mut self, limit: usize) -> Vec<Request> {
        let mut inputs: Vec<Request> = vec![];
        inputs.append(&mut self.input_queue);
        while let Some((buf, amt, src)) = self.port.recv() {
            for subscriber in &mut self.subscribers {
                if subscriber.address == IpAddress::new(&src.ip().to_string(), src.port()) {
                    subscriber.last_contact = Utc::now().timestamp() as u128;
                }
            }
            let msg: Request = match serde_json::from_str(match std::str::from_utf8(&buf[..amt]) {
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
                Request::Ping => {}
                Request::Subscribe(info) => {
                    let mut recognized_subscriber = false;
                    for subscriber in &mut self.subscribers {
                        if subscriber.address == info.address {
                            subscriber.message_kinds = info.message_kinds.clone();
                            recognized_subscriber = true;
                        }
                    }
                    if !recognized_subscriber {
                        logger::log(
                            format!(
                                "New subscriber: {} at [{}] subscribing to {:?}.",
                                info.identifier.str(),
                                info.address,
                                info.message_kinds
                            ),
                            LogContext::Network,
                            LogKind::Note,
                        );
                        self.subscribers.push(info);
                    }
                    self.publish_subscribers();
                    self.input_queue.push(Request::NotifySubscribers);
                }
                Request::Unsubscribe(info) => {
                    self.subscribers = self
                        .subscribers
                        .clone()
                        .into_iter()
                        .filter(|sub| sub.address != info.address)
                        .collect();
                    self.publish_subscribers();
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

    fn notify_multiple(&mut self, notifications: Vec<Message>) {
        for notif in notifications {
            self.notify(notif);
        }
    }

    fn notify(&mut self, notification: Message) {
        if false && notification.to_type() != MessageType::TransportData {
            logger::log(
                format!("Sending network message: {notification:?}"),
                LogContext::Network,
                LogKind::Debug,
            );
        }
        self.subscribers = self
            .subscribers
            .clone()
            .into_iter()
            .filter(|sub| {
                Utc::now()
                    .signed_duration_since(
                        DateTime::from_timestamp_secs(sub.last_contact as i64).unwrap_or_default(),
                    )
                    .num_minutes()
                    < 15
            })
            .collect();

        for subscriber in &self.subscribers {
            if subscriber.message_kinds.contains(notification.to_type()) {
                self.port.send_to(
                    serde_json::to_string(&notification)
                        .expect("notification has trivial derived conversion")
                        .as_bytes(),
                    SocketAddr::new(
                        IpAddr::from_str(&subscriber.address.addr_as_str())
                            .expect("all subscriber addresses are santizied earlier"),
                        subscriber.address.port,
                    ),
                );
            }
        }
    }
}
