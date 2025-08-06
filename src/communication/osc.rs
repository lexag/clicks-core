use crate::communication::{interface::CommunicationInterface, netport::NetworkPort};
use crate::logger;
use common::command::ControlCommand;
use common::status::Notification;
use common::{control::ControlMessage, network::SubscriberInfo};
use jack::NotificationHandler;
use rosc::address::{Matcher, OscAddress};
use rosc::decoder::decode_udp;
use rosc::{OscBundle, OscError, OscMessage, OscPacket, OscTime, OscType};
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
            let data = buf.clone();
            match self.handle_bytes(&data, amt) {
                Ok(mut cc) => self.input_queue.append(&mut cc),
                Err(err) => {}
            }
            self.last_recv_src = src;
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
            matcher: Matcher::new("/null").expect("Constant pattern cannot fail"),
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

    fn handle_bytes(&mut self, buf: &[u8], amt: usize) -> Result<Vec<ControlMessage>, OscError> {
        let (amt, packet) = decode_udp(&buf[..amt])?;
        self.handle_packet(packet)
    }

    fn handle_packet(&mut self, packet: OscPacket) -> Result<Vec<ControlMessage>, OscError> {
        match packet {
            OscPacket::Bundle(bundle) => self.handle_bundle(bundle),
            OscPacket::Message(msg) => self.handle_message(msg),
        }
    }

    fn handle_bundle(&mut self, bundle: OscBundle) -> Result<Vec<ControlMessage>, OscError> {
        if bundle.timetag
            <= OscTime::try_from(SystemTime::now()).expect("SystemTime is after Unix Epoch")
        {
            let mut cmds = vec![];
            for packet in bundle.content {
                cmds.append(&mut self.handle_packet(packet)?);
            }
            return Ok(cmds);
        } else {
            self.bundle_pool.push(bundle);
        }
        return Ok(Vec::new());
    }

    fn step_address(&mut self) -> &str {
        // Split out the first word in the address to do breadth first search on
        let res = self.address.split_once('/');

        // This deals with empty or single word addresses like "oscillator"
        let (address_space, address) = match res {
            None => {
                self.address_space = self.address.clone();
                return self.address_space.as_str();
            }
            Some(val) => val,
        };

        // This deals with misshapen addresses like "oscillator/"
        if address.is_empty() {
            self.address_space = self.address.clone();
            return self.address_space.as_str();
        }

        self.address_space = address_space.to_string();
        self.address = address.to_string();

        return self.address_space.as_str();
    }

    fn handle_message(&mut self, message: OscMessage) -> Result<Vec<ControlMessage>, OscError> {
        let mut msg = message.clone();
        // Valid osc addresses have "/" as the first character.
        // It can be discarded once we know it's there.
        if msg.addr.is_empty() || msg.addr.remove(0) != '/' {
            return Err(OscError::BadAddress(message.addr));
        }

        self.args = msg.args;
        self.address = msg.addr;

        return match self.step_address() {
            "control" => match self.step_address() {
                "transport" => self.addr_control_transport_(),
                "cue" => self.addr_control_cue_(),
                _ => Err(OscError::Unimplemented),
            },
            "edit" => match self.step_address() {
                "channel" => self.addr_edit_channel_(),
                "config" => self.addr_edit_config_(),
                _ => Err(OscError::Unimplemented),
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
                    Ok(vec![])
                } else {
                    Err(OscError::BadArg("subscriber".to_string()))
                }
            }
            _ => Err(OscError::Unimplemented),
        };
    }

    fn send_message(&mut self, msg: OscMessage) {
        self.send_packet(OscPacket::Message(msg));
    }

    fn send_messages(&mut self, messages: Vec<OscMessage>) {
        self.send_packet(OscPacket::Bundle(OscBundle {
            timetag: OscTime::try_from(SystemTime::now()).expect("SystemTime is after Unix Epoch"),
            content: messages
                .iter()
                .map(|m| OscPacket::Message(m.clone()))
                .collect(),
        }));
    }

    fn send_packet(&mut self, packet: OscPacket) {
        for subscriber in self.subscribers.clone() {
            self.port.send_to(
                match &rosc::encoder::encode(&packet) {
                    Ok(val) => val.as_slice(),
                    Err(err) => continue,
                },
                subscriber,
            );
        }
    }

    fn addr_control_transport_(&mut self) -> Result<Vec<ControlMessage>, OscError> {
        return match self.step_address() {
            "start" => Ok(vec![ControlMessage::ControlCommand(
                ControlCommand::TransportStart,
            )]),
            "stop" => Ok(vec![ControlMessage::ControlCommand(
                ControlCommand::TransportStop,
            )]),
            "zero" => Ok(vec![ControlMessage::ControlCommand(
                ControlCommand::TransportZero,
            )]),
            "seek" => {
                if let Some(dest) = self.get_arg(0).int() {
                    Ok(vec![ControlMessage::ControlCommand(
                        ControlCommand::TransportSeekBeat(dest as usize),
                    )])
                } else {
                    Err(OscError::BadArg("beat index".to_string()))
                }
            }
            "jump" => {
                if let Some(dest) = self.get_arg(0).int() {
                    Ok(vec![ControlMessage::ControlCommand(
                        ControlCommand::TransportJumpBeat(dest as usize),
                    )])
                } else {
                    Err(OscError::BadArg("beat index".to_string()))
                }
            }
            _ => Err(OscError::Unimplemented),
        };
    }

    fn addr_control_cue_(&mut self) -> Result<Vec<ControlMessage>, OscError> {
        return match self.step_address() {
            "+" => Ok(vec![ControlMessage::ControlCommand(
                ControlCommand::LoadNextCue,
            )]),
            "-" => Ok(vec![ControlMessage::ControlCommand(
                ControlCommand::LoadPreviousCue,
            )]),
            "load" => {
                if let Some(cue_idx) = self.get_arg(0).int() {
                    Ok(vec![ControlMessage::ControlCommand(
                        ControlCommand::LoadCueByIndex(cue_idx as usize),
                    )])
                } else {
                    Err(OscError::BadArg("cue index".to_string()))
                }
            }
            _ => Err(OscError::Unimplemented),
        };
    }

    fn addr_edit_channel_(&mut self) -> Result<Vec<ControlMessage>, OscError> {
        if let Ok(matcher) = Matcher::new(&format!("/{}", self.address)) {
            self.matcher = matcher;
        } else {
            return Err(OscError::BadAddress(self.address.clone()));
        }

        let mut cmds = vec![];
        for chidx in 0..32 {
            if self.addreq(format!("/{chidx}/gain"))
                && let Some(gain) = self.get_arg(0).float()
            {
                cmds.push(ControlMessage::ControlCommand(
                    ControlCommand::SetChannelGain(chidx, gain),
                ));
            }
            if self.addreq(format!("/{chidx}/mute"))
                && let Some(mute) = self.get_arg(0).bool()
            {
                cmds.push(ControlMessage::ControlCommand(
                    ControlCommand::SetChannelMute(chidx, mute),
                ));
            }
            for out_idx in 0..64 {
                if self.addreq(format!("/{chidx}/route/{out_idx}"))
                    && let Some(patch) = self.get_arg(0).bool()
                {
                    cmds.push(ControlMessage::RoutingChangeRequest(chidx, out_idx, patch));
                }
            }
        }
        return Ok(cmds);
    }

    fn addr_edit_config_(&mut self) -> Result<Vec<ControlMessage>, OscError> {
        Err(OscError::Unimplemented)
    }

    fn get_arg(&mut self, idx: usize) -> OscType {
        return self.args.get(idx).cloned().unwrap_or(OscType::Nil);
    }

    fn addreq(&self, addr: String) -> bool {
        return self.matcher.match_address(match &OscAddress::new(addr) {
            Ok(val) => val,
            Err(err) => return false,
        });
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
            Notification::CueChanged(cue) => {
                vec![
                    osc_msg("/notification/cue/index", OscType::Int(cue.cue_idx as i32)),
                    osc_msg(
                        "/notification/cue/length",
                        OscType::Int(cue.cue.get_beats().len() as i32),
                    ),
                    osc_msg(
                        "/notification/cue/ident",
                        OscType::String(cue.cue.metadata.human_ident),
                    ),
                    osc_msg(
                        "/notification/cue/name",
                        OscType::String(cue.cue.metadata.name),
                    ),
                ]
            }
            Notification::BeatChanged(state) => {
                vec![
                    osc_msg(
                        "/notification/transport/beat/index",
                        OscType::Int(state.beat_idx.try_into().unwrap_or(0)),
                    ),
                    osc_msg(
                        "/notification/transport/beat/count",
                        OscType::Int(state.beat.count.try_into().unwrap_or(0)),
                    ),
                    osc_msg(
                        "/notification/transport/beat/bar",
                        OscType::Int(state.beat.bar_number.try_into().unwrap_or(0)),
                    ),
                ]
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_osc() {
        let mut handler = OscNetHandler::new(0);
        let invalids = ["", "\0\0\0\0\0\0\0", "agewqaehgo"];
        for invalid in invalids {
            let result = handler.handle_bytes(invalid.as_bytes(), invalid.len());
            assert!(
                result.is_err(),
                "Bytes '{}' is not handled correctly. Actual result: {:?}",
                invalid,
                result.unwrap_err()
            );
        }

        let invalids = ["/test//", "test", "/"];
        for invalid in invalids {
            let packet = OscPacket::Message(OscMessage {
                addr: invalid.to_string(),
                args: vec![OscType::Int(1)],
            });
            let result = handler.handle_bytes(invalid.as_bytes(), invalid.len());
            assert!(
                result.is_err(),
                "Address '{}' is not handled correctly. Actual result: {:?}",
                invalid,
                result.unwrap_err()
            );
        }
    }
    #[test]
    fn valid_osc() {
        let cases = vec![
            (
                "/control/transport/start",
                vec![],
                vec![ControlMessage::ControlCommand(
                    ControlCommand::TransportStart,
                )],
            ),
            (
                "/control/transport/seek",
                vec![OscType::Int(5)],
                vec![ControlMessage::ControlCommand(
                    ControlCommand::TransportSeekBeat(5),
                )],
            ),
            (
                "/edit/channel/{1,2}/gain",
                vec![OscType::Float(0.2)],
                vec![
                    ControlMessage::ControlCommand(ControlCommand::SetChannelGain(1, 0.2)),
                    ControlMessage::ControlCommand(ControlCommand::SetChannelGain(2, 0.2)),
                ],
            ),
            (
                "/edit/channel/?/gain",
                vec![OscType::Float(0.2)],
                vec![
                    ControlMessage::ControlCommand(ControlCommand::SetChannelGain(0, 0.2)),
                    ControlMessage::ControlCommand(ControlCommand::SetChannelGain(1, 0.2)),
                    ControlMessage::ControlCommand(ControlCommand::SetChannelGain(2, 0.2)),
                    ControlMessage::ControlCommand(ControlCommand::SetChannelGain(3, 0.2)),
                    ControlMessage::ControlCommand(ControlCommand::SetChannelGain(4, 0.2)),
                    ControlMessage::ControlCommand(ControlCommand::SetChannelGain(5, 0.2)),
                    ControlMessage::ControlCommand(ControlCommand::SetChannelGain(6, 0.2)),
                    ControlMessage::ControlCommand(ControlCommand::SetChannelGain(7, 0.2)),
                    ControlMessage::ControlCommand(ControlCommand::SetChannelGain(8, 0.2)),
                    ControlMessage::ControlCommand(ControlCommand::SetChannelGain(9, 0.2)),
                ],
            ),
        ];

        let mut handler = OscNetHandler::new(0);
        for (addr, args, expected) in cases {
            let result = handler
                .handle_packet(OscPacket::Message(OscMessage {
                    addr: addr.to_string(),
                    args,
                }))
                .expect("Assert Ok");
            assert_eq!(result, expected);
        }
    }
}
