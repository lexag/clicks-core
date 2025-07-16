use crate::logger;
use common::{network::SubscriberInfo, status::Notification};
use local_ip_address::local_ip;
use std::net::{SocketAddr, UdpSocket};

const BUFFER_SIZE: usize = 1024 * 64;

#[derive(Debug)]
pub struct NetworkPort {
    socket: UdpSocket,
    buffer: [u8; BUFFER_SIZE],
}

impl NetworkPort {
    pub fn new(port: usize) -> Self {
        let s = Self {
            buffer: [0; BUFFER_SIZE],
            socket: UdpSocket::bind(format!("{}:{}", local_ip().unwrap().to_string(), port))
                .expect("couldn't open local port"),
        };
        let _ = s.socket.set_nonblocking(true);
        return s;
    }

    pub fn recv(&mut self) -> Option<(&[u8; BUFFER_SIZE], usize, SocketAddr)> {
        match self.socket.recv_from(&mut self.buffer) {
            Ok((amt, src)) => return Some((&self.buffer, amt, src)),
            Err(err) => None,
        }
    }

    pub fn send_to(&mut self, content: &[u8], address: SocketAddr) {
        match self.socket.send_to(content, address) {
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
