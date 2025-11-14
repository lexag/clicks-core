#![warn(clippy::all)]

mod audio;
mod boot;
mod cbnet;
mod communication;
mod hardware;
mod logger;

use crate::{
    audio::{
        handler::AudioHandler, metronome::Metronome, playback::PlaybackHandler,
        timecode::TimecodeSource,
    },
    cbnet::CrossbeamNetwork,
    communication::{
        interface::CommunicationInterface, jsonnet::JsonNetHandler, osc::OscNetHandler,
    },
};
use clap::Parser;
use common::{
    cue::{Show, ShowBuilder},
    local::config::{LogContext, LogKind},
    mem::str::String8,
    protocol::{
        message::{Heartbeat, Message},
        request::{ControlAction, Request},
    },
};
use std::{
    path::PathBuf,
    str::FromStr,
    time::{Duration, Instant},
};

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    #[arg(short, long, default_value_t = '-')]
    manual_boot: char,

    #[arg(long, default_value_t = String::from(""))]
    show_path_override: String,
}

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() {
    logger::init();

    #[cfg(feature = "i2c-ui")]
    {
        std::thread::sleep(Duration::from_secs(2));
        hardware::display::ask_usb();
        if hardware::input::wait_yes_no() {
            hardware::usb::mount();
            if boot::get_usb_update_path().is_ok_and(|p| p.try_exists().is_ok_and(|b| b)) {
                hardware::display::ask_patch();
                if hardware::input::wait_yes_no() {
                    if boot::try_patch() {
                        hardware::display::patch_success();
                        std::thread::sleep(Duration::from_secs(2));
                    } else {
                        hardware::display::patch_failure();
                        return;
                    }
                }
            };
            if boot::get_usb_show_path().is_ok_and(|p| p.try_exists().is_ok_and(|b| b)) {
                hardware::display::ask_copy_show();
                if hardware::input::wait_yes_no() {
                    if boot::try_load_usb_show() {
                        hardware::display::generic_success();
                    } else {
                        hardware::display::generic_failure(
                            "Could not copy show from usb".to_string(),
                        );
                    }
                    std::thread::sleep(Duration::from_secs(3));
                }
            };
            hardware::usb::unmount();
        }
    }

    let show_path = match boot::get_show_path() {
        Ok(val) => val,
        Err(err) => {
            boot::log_boot_error(err);
            return;
        }
    };
    // FIXME: ugly way to make sure that jackd is dead after last debug run
    // should not need to exist in normal operation, because power cycle will reset jackd anyway,
    // and that is the only in-use way to rerun the program
    if let Ok(mut child) = std::process::Command::new("killall").arg("jackd").spawn() {
        child.wait();
    }

    let mut config = boot::get_config().expect("required to continue");
    let show = match Show::from_file(boot::get_show_path().unwrap_or_default().join("show.json")) {
        Ok(show) => {
            #[cfg(feature = "i2c-ui")]
            hardware::display::show_load_success(show.clone());
            show
        }
        Err(err) => {
            #[cfg(feature = "i2c-ui")]
            hardware::display::show_load_failure(err);
            Show {
                metadata: ShowMetadata::default(),
                cues: vec![Cue::example(), Cue::example_loop()],
            }
        }
    };
    #[cfg(feature = "i2c-ui")]
    {
        std::thread::sleep(Duration::from_secs(5));
        hardware::display::startup();
    }

    let cbnet = CrossbeamNetwork::new();

    let mut pbh = PlaybackHandler::new(show_path.clone(), 30);
    let mut ah = AudioHandler::new(32, cbnet.clone());
    let mut nh = JsonNetHandler::new(8081);
    let mut osch = OscNetHandler::new(8082);

    let mut last_heartbeat_time = Instant::now();
    let mut loop_count = 0;
    let mut run_flag = true;
    while run_flag {
        loop_count += 1;
        // Get a possible Request from network handler
        // and decide how to handle it. Network handler has already handled and consumed
        // network-specific messages.
        for control_message in [nh.get_all_inputs(), osch.get_all_inputs()]
            .iter()
            .flatten()
        {
            println!("{:?}", control_message);
            match control_message.clone() {
                Request::ControlAction(cmd) => {
                    let _ = cbnet.command(cmd.clone());
                    match cmd {
                        ControlAction::LoadCueByIndex(idx) => pbh.load_cue(show.cues[idx as usize]),
                        ControlAction::SetChannelGain(channel, gain) => {
                            config.channels[channel as usize].gain = gain;
                            nh.notify(Message::ConfigurationChanged(config.clone()));
                        }
                        _ => {}
                    }
                }
                Request::ChangeRouting(a, b, connect) => {
                    ah.try_route_ports(a, b, connect);
                    nh.notify(Message::JACKStateChanged(ah.get_jack_status()));
                }
                Request::NotifySubscribers => {
                    let _ = cbnet.command(ControlAction::DumpStatus);
                    nh.notify(Message::JACKStateChanged(ah.get_jack_status()));
                    nh.notify(Message::ConfigurationChanged(config.clone()));
                }
                Request::Shutdown => {
                    boot::write_config(config.clone());
                    logger::log(
                        format!("Shutdown. Goodnight.",),
                        LogContext::Boot,
                        LogKind::Note,
                    );
                    nh.notify(Message::ShutdownOccured);
                    ah.shutdown();
                    run_flag = false;
                    break;
                }

                Request::Initialize => {
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
                        source.set_gain(config.channels[i].gain);
                    }

                    ah.configure(config.audio.clone());
                    ah.start(sources, show.clone());
                    nh.notify(Message::JACKStateChanged(ah.get_jack_status()));
                }

                Request::ChangeConfiguration(conf) => {
                    config.update(conf);
                    nh.notify(Message::ConfigurationChanged(config.clone()));
                }
                _ => {}
            };
        }

        // Get a possible Message from audio processor
        // and send it to network handler to broadcast.
        match cbnet.notif_rx.try_recv() {
            Ok(msg) => {
                nh.notify(msg.clone());
                osch.notify(msg.clone());
            }
            Err(crossbeam_channel::TryRecvError::Empty) => {}
            _ => {}
        }

        if last_heartbeat_time.elapsed().gt(&Duration::from_secs(1)) {
            let heartbeat = Message::Heartbeat(Heartbeat {
                common_version: String8::new(common::VERSION),
                system_version: String8::new(VERSION),
                system_time: chrono::Utc::now().timestamp() as u64,
                cpu_use_audio: ah.get_cpu_use(),
                process_freq_main: loop_count,
            });
            nh.notify(heartbeat.clone());
            osch.notify(heartbeat.clone());
            last_heartbeat_time = Instant::now();
            loop_count = 0;
        }
    }
}
