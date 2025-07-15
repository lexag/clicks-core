use crate::communication::{interface::CommunicationInterface, netport::NetworkPort};
use crate::logger;
use common::command::ControlCommand;
use common::{control::ControlMessage, network::SubscriberInfo};
use rosc::address::Matcher;
use rosc::decoder::*;
use rosc::*;
use std::time::SystemTime;

pub struct OscNetHandler {
    port: NetworkPort,
    input_queue: Vec<ControlMessage>,
    subscribers: Vec<SubscriberInfo>,
    bundle_pool: Vec<OscBundle>,
    address: String,
    address_space: String,
    args: Vec<OscType>,
}

impl CommunicationInterface for OscNetHandler {
    fn get_inputs(&mut self, limit: usize) -> Vec<ControlMessage> {
        let mut inputs: Vec<ControlMessage> = vec![];
        inputs.append(&mut self.input_queue);
        while let Some((buf, amt, src)) = self.port.recv() {
            if let Ok((amt, packet)) = decode_udp(&buf[..amt]) {
                self.handle_packet(packet);
            }
        }
        return inputs;
    }

    fn notify(&mut self, notification: common::status::Notification) {
        todo!()
    }

    fn notify_multiple(&mut self, notifications: Vec<common::status::Notification>) {
        todo!()
    }
}

impl OscNetHandler {
    pub fn new(port: usize) -> Self {
        Self {
            port: NetworkPort::new(port),
            input_queue: vec![],
            subscribers: vec![],
            bundle_pool: vec![],
            address: String::new(),
            address_space: String::new(),
            args: vec![],
        }
    }

    fn handle_packet(&mut self, packet: OscPacket) {
        match packet {
            OscPacket::Bundle(bundle) => self.handle_bundle(bundle),
            OscPacket::Message(msg) => self.handle_message(msg),
        }
    }

    fn handle_bundle(&mut self, bundle: OscBundle) {
        if bundle.timetag <= OscTime::try_from(SystemTime::now()).unwrap() {
            for packet in bundle.content {
                self.handle_packet(packet);
            }
        } else {
            self.bundle_pool.push(bundle);
        }
    }

    fn step_address(&mut self) -> &str {
        // Split out the first word in the address to do breadth first search on
        let res = self.address.split_once('/');

        // This deals with empty or single word addresses like "oscillator"
        if let None = res {
            self.address_space = self.address.clone();
            return self.address_space.as_str();
        }

        let (address_space, address) = res.unwrap();

        // This deals with misshapen addresses like "oscillator/"
        if address.is_empty() {
            self.address_space = self.address.clone();
            return self.address_space.as_str();
        }

        self.address_space = address_space.to_string();
        self.address = address.to_string();

        return self.address_space.as_str();
    }

    fn handle_message(&mut self, message: OscMessage) {
        let mut msg = message.clone();
        // Valid osc addresses have "/" as the first character.
        // It can be discarded once we know it's there.
        if msg.addr.is_empty() || msg.addr.remove(0) != '/' {
            return;
        }

        self.args = msg.args;
        self.address = msg.addr;

        match self.step_address() {
            "control" => match self.step_address() {
                "transport" => self.addr_control_transport_(),
                "cue" => self.addr_control_cue_(),
                _ => {}
            },
            "edit" => match self.step_address() {
                "channel" => self.addr_edit_channel_(),
                "config" => self.addr_edit_config_(),
                _ => {}
            },
            _ => {
                return;
            }
        }
    }

    fn addr_control_transport_(&mut self) {
        match self.step_address() {
            "start" => self.input_queue.push(ControlMessage::ControlCommand(
                ControlCommand::TransportStart,
            )),
            "stop" => self.input_queue.push(ControlMessage::ControlCommand(
                ControlCommand::TransportStop,
            )),
            "zero" => self.input_queue.push(ControlMessage::ControlCommand(
                ControlCommand::TransportZero,
            )),
            _ => return,
        }
    }

    fn addr_control_cue_(&mut self) {}

    fn addr_edit_channel_(&mut self) {}

    fn addr_edit_config_(&mut self) {}
}
