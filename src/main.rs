#![warn(clippy::all, rust_2018_idioms)]

mod audio;
mod boot;
mod logger;
mod metronome;
mod network;
mod playback;
mod timecode;

use common::{
    self, command::ControlCommand, config::BootProgramOrder, control::ControlMessage, cue::Cue,
    network::JACKStatus, show::Show, status::Notification,
};

use crate::{audio::handler::AudioHandler, playback::PlaybackHandler};
use clap::Parser;
use crossbeam_channel::{unbounded, Receiver, Sender};
use metronome::Metronome;
use network::NetworkHandler;
use std::{path::PathBuf, str::FromStr};
use timecode::TimecodeSource;

#[derive(Clone)]
pub struct CrossbeamNetwork {
    pub cmd_tx: Sender<ControlCommand>,
    pub cmd_rx: Receiver<ControlCommand>,
    pub status_tx: Sender<Notification>,
    pub status_rx: Receiver<Notification>,
}

impl CrossbeamNetwork {
    fn new() -> Self {
        let (cmd_tx, cmd_rx): (Sender<ControlCommand>, Receiver<ControlCommand>) = unbounded();
        let (status_tx, status_rx): (Sender<Notification>, Receiver<Notification>) = unbounded();
        Self {
            cmd_tx,
            cmd_rx,
            status_tx,
            status_rx,
        }
    }
}

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    #[arg(short, long, default_value_t = '-')]
    manual_boot: char,

    #[arg(long, default_value_t = String::from(""))]
    show_path_override: String,
}

fn main() {
    logger::init();
    let args = Args::parse();

    let show_path = if args.show_path_override.is_empty() {
        match boot::find_show_path() {
            Ok(val) => val,
            Err(err) => {
                boot::log_boot_error(err);
                return;
            }
        }
    } else {
        match PathBuf::from_str(&args.show_path_override) {
            Ok(val) => val,
            Err(err) => panic!("Incorrect path argument to show_path_override."),
        }
    };

    // FIXME: ugly way to make sure that jackd is dead after last debug run
    // should not need to exist in normal operation, because power cycle will reset jackd anyway,
    // and that is the only in-use way to rerun the program
    let _ = std::process::Command::new("killall")
        .arg("jackd")
        .spawn()
        .unwrap()
        .wait();

    let mut config = boot::get_config().unwrap();
    let show = Show::from_file(show_path.join("show.json")).unwrap();

    let cbnet = CrossbeamNetwork::new();

    let mut pbh = PlaybackHandler::new(show_path.clone(), 30);
    let mut ah = AudioHandler::new(32, cbnet.clone());
    let mut nh = NetworkHandler::new("8081");
    nh.start();

    let mut status_counter: u8 = 0;
    loop {
        // Get a possible ControlMessage from network handler
        // and decide how to handle it. Network handler has already handled and consumed
        // network-specific messages.
        let control_message = nh.tick();
        match control_message {
            None => {}
            Some(msg) => match msg {
                ControlMessage::ControlCommand(cmd) => {
                    let _ = cbnet.cmd_tx.send(cmd.clone());
                    match cmd {
                        ControlCommand::LoadCue(cue) => pbh.load_cue(cue),
                        ControlCommand::LoadShow(show) => pbh.load_show(show),
                        ControlCommand::SetChannelGain(channel, gain) => {
                            config.channels.channels[channel].gain = gain;
                            nh.send_to_all(Notification::ConfigurationChanged(config.clone()));
                        }
                        _ => {}
                    }
                }
                ControlMessage::RoutingChangeRequest(a, b, connect) => {
                    ah.try_route_ports(a, b, connect);
                    nh.send_to_all(Notification::JACKStateChanged(ah.get_jack_status()));
                }
                ControlMessage::NotifySubscribers => {
                    let _ = cbnet.cmd_tx.send(ControlCommand::DumpStatus);
                    nh.send_to_all(Notification::JACKStateChanged(ah.get_jack_status()));
                    nh.send_to_all(Notification::ConfigurationChanged(config.clone()));
                }
                ControlMessage::Shutdown => {
                    boot::write_config(config);
                    logger::log(
                        format!("Shutdown. Goodnight.",),
                        logger::LogContext::Boot,
                        logger::LogKind::Note,
                    );
                    nh.send_to_all(Notification::ShutdownOccured);
                    ah.shutdown();
                    break;
                }

                ControlMessage::Initialize => {
                    let mut sources = vec![
                        audio::source::SourceConfig::new(
                            "metronome".to_string(),
                            Box::new(Metronome::new()),
                        ),
                        audio::source::SourceConfig::new(
                            "timecode".to_string(),
                            Box::new(TimecodeSource::new(25)),
                        ),
                    ];
                    pbh.load_show(show.clone());
                    sources.extend(pbh.create_audio_sources());
                    for (i, source) in sources.iter_mut().enumerate() {
                        source.set_gain(config.channels.channels[i].gain);
                    }

                    ah.configure(config.audio.clone());
                    ah.start(sources);
                    nh.send_to_all(Notification::JACKStateChanged(ah.get_jack_status()));
                    let _ = cbnet
                        .cmd_tx
                        .try_send(ControlCommand::LoadShow(show.clone()));
                }

                ControlMessage::SetConfigurationRequest(conf) => {
                    config = conf;
                    nh.send_to_all(Notification::ConfigurationChanged(config.clone()));
                }
                _ => {}
            },
        };

        // Get a possible Notification from audio processor
        // and send it to network handler to broadcast.
        match cbnet.status_rx.try_recv() {
            Ok(msg) => {
                status_counter += 1;
                match msg {
                    Notification::TransportChanged(status) => {
                        if status.clone().running || status_counter > 16 {
                            nh.send_to_all(Notification::TransportChanged(status));
                            status_counter = 0;
                        }
                    }
                    _ => {
                        nh.send_to_all(msg.clone());
                    }
                }
            }
            Err(crossbeam_channel::TryRecvError::Empty) => {}
            _ => {}
        }
    }
}
