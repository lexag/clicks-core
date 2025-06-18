mod audio;
mod metronome;
mod timecode;

use std::net::UdpSocket;

use common::{
    self,
    command::ControlCommand,
    network::{ControlMessageKind, StatusMessageKind, SubscriberInfo},
    status::ProcessStatus,
};

use crossbeam_channel::{unbounded, Receiver, Sender};
use metronome::Metronome;
use timecode::TimecodeSource;

fn main() {
    let br = common::cue::Cue::example_loop();
    let sources = vec![
        audio::source::SourceConfig {
            name: "metronome".to_string(),
            connections: vec!["playback_1".to_string(), "playback_2".to_string()],
            source_device: Box::new(Metronome::new()),
        },
        audio::source::SourceConfig {
            name: "timecode".to_string(),
            connections: vec![],
            source_device: Box::new(TimecodeSource::new(25)),
        },
    ];
    let (cmd_tx, cmd_rx): (Sender<ControlCommand>, Receiver<ControlCommand>) = unbounded();
    let (status_tx, status_rx): (Sender<ProcessStatus>, Receiver<ProcessStatus>) = unbounded();
    let ah = audio::handler::AudioHandler::new(
        audio::config::AudioConfig {
            client_name: "clicks-rust".to_string(),
            system_name: "system".to_string(),
        },
        sources,
        cmd_rx,
        status_tx,
    );
    let _ = cmd_tx.send(ControlCommand::TransportStop);
    let _ = cmd_tx.send(ControlCommand::LoadCue(br));
    let _ = cmd_tx.send(ControlCommand::TransportZero);
    let _ = cmd_tx.send(ControlCommand::TransportStart);

    let socket = UdpSocket::bind("127.0.0.1:8081").unwrap();
    let _ = socket.set_nonblocking(true);

    let mut subscribers: Vec<SubscriberInfo> = vec![];

    loop {
        match status_rx.try_recv() {
            Ok(status) => {
                for subscriber in &subscribers {
                    let _ = socket.send_to(
                        serde_json::to_string(&StatusMessageKind::ProcessStatus(Some(
                            status.clone(),
                        )))
                        .unwrap()
                        .as_bytes(),
                        format!("{}:{}", subscriber.address, subscriber.port),
                    );
                }
            }

            // If channel is empty, continue with process
            Err(crossbeam_channel::TryRecvError::Empty) => {}
            _ => {}
        }

        let mut buf = [0; 1024];
        match socket.recv_from(&mut buf) {
            Ok((amt, _src)) => {
                let msg: ControlMessageKind =
                    match serde_json::from_str(std::str::from_utf8(&buf[..amt]).unwrap()) {
                        Ok(msg) => msg,
                        Err(err) => {
                            panic!(
                                "failed parse! {err} \n {}",
                                std::str::from_utf8(&buf[..amt]).unwrap()
                            );
                        }
                    };
                match msg {
                    ControlMessageKind::ControlCommand(cmd) => {
                        let _ = cmd_tx.send(cmd);
                    }
                    ControlMessageKind::SubscribeRequest(info) => {
                        println!("New subscriber: {info:?}");
                        subscribers.push(info);
                    }
                    _ => {}
                }
            }
            Err(err) => {}
        }
    }
}
