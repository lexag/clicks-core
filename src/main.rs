#![allow(warnings)]

mod audio;
mod boot;
mod logger;
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

use clap::Parser;
use crossbeam_channel::{unbounded, Receiver, Sender};
use metronome::Metronome;
use network::NetworkHandler;
use std::{path::PathBuf, str::FromStr};
use timecode::TimecodeSource;

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    #[arg(short, long, default_value_t = '-')]
    manual_boot: char,

    #[arg(long, default_value_t = String::from(""))]
    config_path_override: String,
}

fn main() {
    logger::init();
    let args = Args::parse();

    let config_path = if args.config_path_override.is_empty() {
        match boot::find_config_path() {
            Ok(val) => val,
            Err(err) => {
                boot::log_boot_error(err);
                return;
            }
        }
    } else {
        match PathBuf::from_str(&args.config_path_override) {
            Ok(val) => val,
            Err(err) => panic!("Incorrect path argument to config_path_override."),
        }
    };

    let boot_order = match args.manual_boot {
        'c' => common::config::BootProgramOrder::WriteConfig,
        'u' => common::config::BootProgramOrder::Upgrade,
        'l' => common::config::BootProgramOrder::ExtractLogs,
        _ => match boot::get_config(config_path.clone()) {
            Ok(val) => val.boot_order,
            Err(err) => {
                boot::log_boot_error(err);
                return;
            }
        },
    };

    match boot_order {
        common::config::BootProgramOrder::WriteConfig => {
            if let Err(err) = boot::write_default_config(config_path.clone()) {
                boot::log_boot_error(err);
            }
            return;
        }
        common::config::BootProgramOrder::Upgrade => {
            todo!("Bootstrapping updates is not yet implemented.")
        }
        common::config::BootProgramOrder::ExtractLogs => {
            if let Err(err) = boot::copy_logs(config_path.clone()) {
                boot::log_boot_error(err);
            }
            return;
        }
        common::config::BootProgramOrder::Run => {
            let config = boot::get_config(config_path).unwrap();
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
            let mut ah = audio::handler::AudioHandler::new(
                config.audio.unwrap(),
                sources,
                cmd_rx,
                cmd_tx.clone(),
                status_tx,
            );

            let _ = cmd_tx.send(ControlCommand::LoadShow(config.show.unwrap()));
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
                            nh.send_to_all(StatusMessageKind::JACKStatus(Some(
                                ah.get_jack_status(),
                            )));
                        }
                        ControlMessageKind::NotifySubscribers => {
                            let _ = cmd_tx.send(ControlCommand::DumpStatus);
                            nh.send_to_all(StatusMessageKind::JACKStatus(Some(
                                ah.get_jack_status(),
                            )));
                        }
                        ControlMessageKind::Shutdown => {
                            logger::log(
                                format!("Shutdown. Goodnight.",),
                                logger::LogContext::Boot,
                                logger::LogKind::Note,
                            );
                            nh.send_to_all(StatusMessageKind::Shutdown);
                            ah.shutdown();
                            break;
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
                            _ => {}
                        }
                    }

                    // If channel is empty, continue with process
                    Err(crossbeam_channel::TryRecvError::Empty) => {}
                    _ => {}
                }
            }
        }
    }
}
