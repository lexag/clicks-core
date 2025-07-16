use crate::communication::{interface::CommunicationInterface, netport::NetworkPort};
use crate::logger;
use common::command::ControlCommand;
use common::{control::ControlMessage, network::SubscriberInfo};
use rosc::address::{Matcher, OscAddress};
use rosc::decoder::*;
use rosc::*;
use std::time::SystemTime;

// Valid OSC addresses:
// /control/
//      transport/
//          start
//          stop
//          zero
//          seek i32
//          jump i32
//      cue/
//          +
//          -
//          load i32
//  /edit/
//      channel/
//          {idx}/
//              gain f32
//              mute bool
//              route/
//                  {to} bool
//      route/
//          {from}/
//              {to}/
//                  set bool
//                  toggle
//      config/
//          ...

pub struct OscNetHandler {
    port: NetworkPort,
    input_queue: Vec<ControlMessage>,
    subscribers: Vec<SubscriberInfo>,
    bundle_pool: Vec<OscBundle>,
    matcher: Matcher,
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
            matcher: Matcher::new("/null").unwrap(),
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

        println!("{} | {}", self.address_space, self.address);
        return self.address_space.as_str();
    }

    fn handle_message(&mut self, message: OscMessage) {
        let mut msg = message.clone();
        // Valid osc addresses have "/" as the first character.
        // It can be discarded once we know it's there.
        if msg.addr.is_empty() || msg.addr.remove(0) != '/' {
            return;
        }

        println!("{}, {:?}", message.addr, message.args);

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
            "seek" => {
                if let Some(dest) = self.get_arg(0).int() {
                    self.input_queue.push(ControlMessage::ControlCommand(
                        ControlCommand::TransportSeekBeat(dest as usize),
                    ));
                }
            }
            "jump" => {
                if let Some(dest) = self.get_arg(0).int() {
                    self.input_queue.push(ControlMessage::ControlCommand(
                        ControlCommand::TransportJumpBeat(dest as usize),
                    ));
                }
            }
            _ => {}
        }
    }

    fn addr_control_cue_(&mut self) {
        match self.step_address() {
            "+" => self
                .input_queue
                .push(ControlMessage::ControlCommand(ControlCommand::LoadNextCue)),
            "-" => self.input_queue.push(ControlMessage::ControlCommand(
                ControlCommand::LoadPreviousCue,
            )),
            "load" => {
                if let Some(cue_idx) = self.get_arg(0).int() {
                    self.input_queue.push(ControlMessage::ControlCommand(
                        ControlCommand::LoadCueByIndex(cue_idx as usize),
                    ));
                }
            }
            _ => {}
        }
    }

    fn addr_edit_channel_(&mut self) {
        if let Ok(matcher) = Matcher::new(&format!("/{}", self.address)) {
            self.matcher = matcher;
        } else {
            return;
        }
        println!("{}", self.address);

        for chidx in 0..32 {
            if self.addreq(format!("/{chidx}/gain"))
                && let Some(gain) = self.get_arg(0).float()
            {
                self.input_queue.push(ControlMessage::ControlCommand(
                    ControlCommand::SetChannelGain(chidx, gain),
                ));
            }
            if self.addreq(format!("/{chidx}/mute"))
                && let Some(mute) = self.get_arg(0).bool()
            {
                self.input_queue.push(ControlMessage::ControlCommand(
                    ControlCommand::SetChannelMute(chidx, mute),
                ));
            }
            for out_idx in 0..64 {
                if self.addreq(format!("/{chidx}/route/{out_idx}"))
                    && let Some(patch) = self.get_arg(0).bool()
                {
                    self.input_queue
                        .push(ControlMessage::RoutingChangeRequest(chidx, out_idx, patch));
                }
            }
        }
    }

    fn addr_edit_config_(&mut self) {}

    fn get_arg(&mut self, idx: usize) -> OscType {
        return self.args.get(idx).cloned().unwrap_or(OscType::Nil);
    }

    fn addreq(&self, addr: String) -> bool {
        return self.matcher.match_address(&OscAddress::new(addr).unwrap());
    }
}
