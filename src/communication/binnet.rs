use std::net::{IpAddr, Ipv4Addr, SocketAddr};

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
    mem::network::{IpAddress, SubscriberInfo},
    protocol::{
        message::{LargeMessage, Message},
        request::Request,
    },
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
            format!("opened binnet port {}", a.port.socket.local_addr().unwrap()),
            LogContext::Network,
            LogKind::Note,
        );
        a
    }

    pub fn publish_subscribers(&mut self) {
        self.notify(Message::Large(LargeMessage::NetworkChanged(
            NetworkStatus {
                subscribers: self.subscribers.clone(),
            },
        )));
    }
}

impl CommunicationInterface for BinaryNetHandler {
    fn get_inputs(&mut self, limit: usize) -> Vec<Request> {
        let mut inputs: Vec<Request> = vec![];
        inputs.append(&mut self.input_queue);
        while let Some((buf, amt, src)) = self.port.recv() {
            for subscriber in &mut self.subscribers {
                if Some(subscriber.address)
                    == IpAddress::from_str_and_port(&src.ip().to_string(), src.port())
                {
                    subscriber.last_contact = Utc::now().timestamp() as u128;
                }
            }
            let msg: Request = match postcard::from_bytes::<Request>(&buf[..amt]) {
                Ok(msg) => msg,
                Err(err) => {
                    panic!(
                        "failed parse! {err} \n {}",
                        std::str::from_utf8(&buf[..amt]).unwrap_or_default()
                    );
                }
            };
            match msg {
                Request::Ping => {}
                Request::Subscribe(info) => {
                    let mut recognized_subscriber = false;
                    for subscriber in &mut self.subscribers {
                        if subscriber.address == info.address {
                            subscriber.message_kinds = info.message_kinds;
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
        inputs
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

        let encoded_result = match notification.clone() {
            Message::Small(message) => postcard::to_stdvec(&message),
            Message::Large(message) => postcard::to_stdvec(&message),
        };

        let mut buffer = match encoded_result {
            Ok(res) => res,
            Err(_err) => return,
        };

        // insert a message size byte at the start, which tells the client if this is a small or
        // large message, since otherwise they can happen to look like the other size, and be
        // parsed incorrectly
        //
        // The LSB of the size byte is enough to tell: 1 is small, 0 is large, but we have some
        // extra redundancy to a) make sure that it is actually a size byte and not a random bit in
        // some misplaced message, and b) to identify the size byte in both flipped and non-flipped
        // ordering
        buffer.insert(
            0,
            match notification {
                Message::Small(..) => 0xE1,
                Message::Large(..) => 0xD2,
            },
        );

        //logger::log(
        //    format!(
        //        "sent Message: {:?}\n {}\n({} bytes)\n",
        //        notification.to_type(),
        //        hex::encode_upper(&buffer),
        //        buffer.len()
        //    ),
        //    LogContext::Network,
        //    LogKind::Debug,
        //);

        for subscriber in &self.subscribers {
            if subscriber.message_kinds.contains(notification.to_type()) {
                self.port.send_to(
                    &buffer,
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
