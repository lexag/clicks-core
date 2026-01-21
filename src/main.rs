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
        binnet::BinaryNetHandler, interface::CommunicationInterface, osc::OscNetHandler,
    },
    logger::{LogDispatcher, LogItem},
};
use common::{
    cue::{Cue, Show, ShowBuilder},
    local::config::{LogContext, LogKind, SystemConfiguration},
    mem::str::StaticString,
    protocol::{
        message::{Heartbeat, LargeMessage, Message, SmallMessage},
        request::{ControlAction, Request},
    },
};
use std::time::{Duration, Instant};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() {
    let cbnet = CrossbeamNetwork::new();
    let log_dispatcher = LogDispatcher::new(cbnet.clone());
    let mut nh = BinaryNetHandler::new(&log_dispatcher, 8081);
    let mut osch = OscNetHandler::new(8082);

    #[cfg(feature = "i2c-ui")]
    {
        std::thread::sleep(Duration::from_secs(2));
        let _ = hardware::display::ask_usb();
        if hardware::input::wait_yes_no() {
            hardware::usb::mount();
            if boot::get_usb_update_path().is_ok_and(|p| p.try_exists().is_ok_and(|b| b)) {
                let _ = hardware::display::ask_patch();
                if hardware::input::wait_yes_no() {
                    if boot::try_patch() {
                        let _ = hardware::display::patch_success();
                        std::thread::sleep(Duration::from_secs(2));
                    } else {
                        let _ = hardware::display::patch_failure();
                        return;
                    }
                }
            };
            if boot::get_usb_show_path().is_ok_and(|p| p.try_exists().is_ok_and(|b| b)) {
                let _ = hardware::display::ask_copy_show();
                if hardware::input::wait_yes_no() {
                    if boot::try_load_usb_show().is_ok() {
                        let _ = hardware::display::generic_success();
                    } else {
                        let _ = hardware::display::generic_failure(
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
            boot::log_boot_error(&log_dispatcher, err);
            return;
        }
    };
    // FIXME: ugly way to make sure that jackd is dead after last debug run
    // should not need to exist in normal operation, because power cycle will reset jackd anyway,
    // and that is the only in-use way to rerun the program
    if let Ok(mut child) = std::process::Command::new("killall").arg("jackd").spawn() {
        let _ = child.wait();
    }

    let mut config = match boot::get_config() {
        Ok(conf) => conf,
        Err(_) => {
            let _ = boot::write_default_config();
            SystemConfiguration::default()
        }
    };
    let show = match ShowBuilder::from_bin_file(
        boot::get_show_path().unwrap_or_default().join("show.bin"),
    ) {
        Ok(show) => {
            #[cfg(feature = "i2c-ui")]
            let _ = hardware::display::show_load_success(&show);
            show
        }
        Err(err) => {
            #[cfg(feature = "i2c-ui")]
            let _ = hardware::display::show_load_failure(&err.to_string());
            let mut show = Show::default();
            show.cues.push(Cue::example());
            show.cues[0].events.pop(0);
            show
        }
    };
    #[cfg(feature = "i2c-ui")]
    {
        std::thread::sleep(Duration::from_secs(5));
        let _ = hardware::display::startup();
    }
    let mut pbh = PlaybackHandler::new(cbnet.clone(), show_path.clone(), 30);
    let mut ah = AudioHandler::new(32, cbnet.clone());

    let mut last_heartbeat_time = Instant::now();
    let mut loop_count = 0;
    let mut run_flag = true;
    let mut cue_idx = 0;
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
            match *control_message {
                Request::ControlAction(cmd) => {
                    cbnet.command(cmd);
                    match cmd {
                        ControlAction::LoadCueByIndex(idx) => {
                            cue_idx = idx;
                            pbh.load_cue(show.cues[cue_idx as usize].clone())
                        }
                        ControlAction::SetChannelGain(channel, gain) => {
                            config.channels[channel as usize].gain = gain;
                            nh.notify(Message::Large(LargeMessage::ConfigurationChanged(config)));
                        }
                        ControlAction::LoadPreviousCue => {
                            if cue_idx > 0 {
                                cue_idx -= 1;
                                cbnet.command(ControlAction::LoadCueByIndex(cue_idx));
                                pbh.load_cue(show.cues[cue_idx as usize].clone())
                            }
                        }
                        ControlAction::LoadNextCue => {
                            if cue_idx as usize + 1 < show.cues.len() {
                                cue_idx += 1;
                                cbnet.command(ControlAction::LoadCueByIndex(cue_idx));
                                pbh.load_cue(show.cues[cue_idx as usize].clone())
                            }
                        }
                        _ => {}
                    }
                }
                Request::ChangeRouting(a, b, connect) => {
                    ah.try_route_ports(a, b, connect);
                    nh.notify(Message::Large(LargeMessage::JACKStateChanged(
                        ah.get_jack_status(),
                    )));
                }
                Request::NotifySubscribers => {
                    cbnet.command(ControlAction::DumpStatus);
                    nh.notify(Message::Large(LargeMessage::JACKStateChanged(
                        ah.get_jack_status(),
                    )));
                    nh.notify(Message::Large(LargeMessage::ConfigurationChanged(config)));
                }
                Request::Shutdown => {
                    let _ = boot::write_config(config);
                    log_dispatcher.log(LogItem::new(
                        "Shutdown. Goodnight.".to_string(),
                        LogContext::Boot,
                        LogKind::Note,
                    ));
                    nh.notify(Message::Small(SmallMessage::ShutdownOccured));
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

                    ah.configure(config.audio);
                    ah.start(sources, show.clone());
                    nh.notify(Message::Large(LargeMessage::JACKStateChanged(
                        ah.get_jack_status(),
                    )));
                }

                Request::ChangeConfiguration(conf) => {
                    config.update(conf);
                    nh.notify(Message::Large(LargeMessage::ConfigurationChanged(config)));
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
            let heartbeat = Message::Small(SmallMessage::Heartbeat(Heartbeat {
                common_version: StaticString::new(common::VERSION),
                system_version: StaticString::new(VERSION),
                system_time: chrono::Utc::now().timestamp() as u64,
                cpu_use_audio: ah.get_cpu_use(),
                process_freq_main: loop_count,
            }));
            nh.notify(heartbeat.clone());
            osch.notify(heartbeat.clone());
            last_heartbeat_time = Instant::now();
            loop_count = 0;
        }
    }
}
