use crate::communication::{interface::CommunicationInterface, netport::NetworkPort};
use crate::logger;
use common::command::ControlCommand;
use common::status::Notification;
use common::{control::ControlMessage, network::SubscriberInfo};
use rosc::address::{Matcher, OscAddress};
use rosc::decoder::*;
use rosc::*;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4};
use std::time::SystemTime;

// Valid control OSC addresses:
// /subscribe i32
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
//
// Valid notification (response) OSC addresses:
//  /notification/
//      transport/
//          running
//          beat/
//              index
//              count
//              bar
//          timecode/
//              h
//              m
//              s
//      cue/
//          index
//          length
//          ident
//          name
//
//

pub struct OscNetHandler {
    port: NetworkPort,
    input_queue: Vec<ControlMessage>,
    subscribers: Vec<SocketAddr>,
    bundle_pool: Vec<OscBundle>,
    matcher: Matcher,
    address: String,
    address_space: String,
    args: Vec<OscType>,
    last_recv_src: SocketAddr,
}

impl CommunicationInterface for OscNetHandler {
    fn get_inputs(&mut self, limit: usize) -> Vec<ControlMessage> {
        let mut inputs: Vec<ControlMessage> = vec![];
        inputs.append(&mut self.input_queue);
        while let Some((buf, amt, src)) = self.port.recv() {
            self.last_recv_src = src;
            if let Ok((amt, packet)) = decode_udp(&buf[..amt]) {
                self.handle_packet(packet);
            }
        }
        return inputs;
    }

    fn notify(&mut self, notification: Notification) {
        for msg in self.notif_to_osc(notification) {
            self.send_message(msg);
        }
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
            last_recv_src: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)), 0),
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
            "subscribe" => {
                if let Some(port) = self
                    .get_arg(0)
                    .int()
                    .unwrap_or_default()
                    .try_into()
                    .unwrap_or_default()
                {
                    self.subscribers
                        .push(SocketAddr::new(self.last_recv_src.ip(), port as u16));
                }
            }
            _ => {
                return;
            }
        }
    }

    fn send_message(&mut self, msg: OscMessage) {
        self.send_packet(OscPacket::Message(msg));
    }

    fn send_messages(&mut self, messages: Vec<OscMessage>) {
        self.send_packet(OscPacket::Bundle(OscBundle {
            timetag: OscTime::try_from(SystemTime::now()).unwrap(),
            content: messages
                .iter()
                .map(|m| OscPacket::Message(m.clone()))
                .collect(),
        }));
    }

    fn send_packet(&mut self, packet: OscPacket) {
        for subscriber in self.subscribers.clone() {
            self.port.send_to(
                rosc::encoder::encode(&packet).unwrap().as_slice(),
                subscriber,
            );
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

    fn notif_to_osc(&mut self, notification: Notification) -> Vec<OscMessage> {
        // Helper function to macro generate osc messages with a single argument
        fn osc_msg(addr: &str, arg: OscType) -> OscMessage {
            return OscMessage {
                addr: addr.to_string(),
                args: vec![arg],
            };
        }

        match notification {
            Notification::CueChanged(idx, cue) => {
                vec![
                    osc_msg("/notification/cue/index", OscType::Int(idx as i32)),
                    osc_msg(
                        "/notification/cue/length",
                        OscType::Int(cue.get_beats().len() as i32),
                    ),
                    osc_msg(
                        "/notification/cue/ident",
                        OscType::String(cue.metadata.human_ident),
                    ),
                    osc_msg("/notification/cue/name", OscType::String(cue.metadata.name)),
                ]
            }
            Notification::BeatChanged(idx, beat) => {
                vec![
                    osc_msg(
                        "/notification/transport/beat/index",
                        OscType::Int(idx.try_into().unwrap_or(0)),
                    ),
                    osc_msg(
                        "/notification/transport/beat/count",
                        OscType::Int(beat.count.try_into().unwrap_or(0)),
                    ),
                    osc_msg(
                        "/notification/transport/beat/bar",
                        OscType::Int(beat.bar_number.try_into().unwrap_or(0)),
                    ),
                ]
            }
            Notification::PlaystateChanged(playstate) => {
                vec![osc_msg(
                    "/notification/transport/running",
                    OscType::Bool(playstate),
                )]
            }
            //          running
            //           {beat/, nextbeat/}
            //              index
            //              count
            //              bar
            //          timecode/
            //              h
            //              m
            //              s
            _ => vec![],
        }
    }
}
