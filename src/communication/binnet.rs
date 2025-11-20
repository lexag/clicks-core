use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    str::FromStr,
};

use crate::{
    communication::{interface::CommunicationInterface, netport::NetworkPort},
    logger,
};
use bincode::config::{BigEndian, Configuration, Fixint};
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

pub struct BinaryNetHandler {
    port: NetworkPort,
    subscribers: Vec<SubscriberInfo>,
    input_queue: Vec<Request>,
}

impl BinaryNetHandler {
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

impl CommunicationInterface for BinaryNetHandler {
    fn get_inputs(&mut self, limit: usize) -> Vec<Request> {
        let mut inputs: Vec<Request> = vec![];
        inputs.append(&mut self.input_queue);
        while let Some((buf, amt, src)) = self.port.recv() {
            for subscriber in &mut self.subscribers {
                if Some(subscriber.address.clone())
                    == IpAddress::from_str_and_port(&src.ip().to_string(), src.port())
                {
                    subscriber.last_contact = Utc::now().timestamp() as u128;
                }
            }
            let config = Configuration::<BigEndian, Fixint>::default()
                .with_big_endian()
                .with_fixed_int_encoding();
            let msg: Request = match bincode::decode_from_slice::<
                Request,
                Configuration<BigEndian, Fixint>,
            >(&buf[..amt], config)
            {
                Ok(msg) => msg.0,
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

        let mut buf = [0u8; size_of::<Message>()];
        let config = Configuration::<BigEndian, Fixint>::default()
            .with_big_endian()
            .with_fixed_int_encoding();
        bincode::encode_into_slice(notification, &mut buf, config)
            .expect("has trivial decode implementation");
        let mut actual_len = 0;
        for (i, byte) in buf.iter().enumerate() {
            if *byte != 0 {
                actual_len = i + 1;
            }
        }
        let actual_buf = &buf[0..actual_len];
        if notification.to_type() != MessageType::TransportData {
            logger::log(
                format!(
                    "Sending network message: {notification:?}\n{:02X?}\nTo {:?}",
                    actual_buf, self.subscribers
                ),
                LogContext::Network,
                LogKind::Debug,
            );
        }

        for subscriber in &self.subscribers {
            if subscriber.message_kinds.contains(notification.to_type()) {
                self.port.send_to(
                    actual_buf,
                    SocketAddr::new(
                        IpAddr::V4(Ipv4Addr::new(
                            subscriber.address.addr[0],
                            subscriber.address.addr[1],
                            subscriber.address.addr[2],
                            subscriber.address.addr[3],
                        )),
                        subscriber.address.port,
                    ),
                );
            }
        }
    }
}
