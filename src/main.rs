mod audio;
mod metronome;
mod network;
mod timecode;

use common::{self, command::ControlCommand, cue::Cue, network::StatusMessageKind, show::Show};

use crossbeam_channel::{unbounded, Receiver, Sender};
use metronome::Metronome;
use network::NetworkHandler;
use timecode::TimecodeSource;

fn main() {
    let show = Show {
        metadata: common::show::ShowMetadata {
            name: "Development Show".to_string(),
            date: "123456".to_string(),
        },
        cues: vec![Cue::example(), Cue::example_loop()],
    };
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
        cmd_tx.clone(),
        status_tx,
    );
    let _ = cmd_tx.send(ControlCommand::TransportStop);
    let _ = cmd_tx.send(ControlCommand::LoadShow(show));
    let _ = cmd_tx.send(ControlCommand::TransportZero);

    let mut nh = NetworkHandler::new("8081", cmd_tx.clone());
    nh.start();

    loop {
        nh.tick();
        match status_rx.try_recv() {
            Ok(msg) => {
                nh.send_to_all(msg.clone());
                match msg {
                    StatusMessageKind::Shutdown => {
                        break;
                    }
                    _ => {}
                }
            }

            // If channel is empty, continue with process
            Err(crossbeam_channel::TryRecvError::Empty) => {}
            _ => {}
        }
    }
}
