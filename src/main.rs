mod audio;
mod metronome;
mod network;
mod timecode;


use common::{
    self,
    command::ControlCommand,
    network::StatusMessageKind,
};

use crossbeam_channel::{unbounded, Receiver, Sender};
use metronome::Metronome;
use network::NetworkHandler;
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
    let (status_tx, status_rx): (Sender<StatusMessageKind>, Receiver<StatusMessageKind>) =
        unbounded();
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

    let mut nh = NetworkHandler::new("8081", cmd_tx.clone());
    nh.start();

    loop {
        nh.tick();
        match status_rx.try_recv() {
            Ok(msg) => {
                nh.send_to_all(msg);
            }

            // If channel is empty, continue with process
            Err(crossbeam_channel::TryRecvError::Empty) => {}
            _ => {}
        }
    }
}
