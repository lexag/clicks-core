mod audio;
mod metronome;
mod network;
mod timecode;

use common::{
    self,
    command::ControlCommand,
    cue::Cue,
    network::{ControlMessageKind, JACKStatus, StatusMessageKind},
    show::Show,
};

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
            source_device: Box::new(Metronome::new()),
        },
        audio::source::SourceConfig {
            name: "timecode".to_string(),
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
            io_size: (2, 2),
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
        // Get a possible ControlMessageKind from network handler
        // and decide how to handle it. Network handler has already handled and consumed
        // network-specific messages.
        let control_message = nh.tick();
        match control_message {
            Some(msg) => match msg {
                ControlMessageKind::ControlCommand(cmd) => {
                    let _ = cmd_tx.send(cmd);
                }
                ControlMessageKind::RoutingChangeRequest(a, b, connect) => {
                    ah.try_route_ports(a, b, connect);
                    nh.send_to_all(StatusMessageKind::JACKStatus(Some(ah.get_jack_status())));
                }
                ControlMessageKind::NotifySubscribers => {
                    let _ = cmd_tx.send(ControlCommand::DumpStatus);
                    nh.send_to_all(StatusMessageKind::JACKStatus(Some(ah.get_jack_status())));
                }
                _ => {}
            },
            None => {}
        };

        // Get a possible StatusMessageKind from audio processor
        // and send it to network handler to broadcast.
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
