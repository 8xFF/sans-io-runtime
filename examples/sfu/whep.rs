use std::{net::SocketAddr, time::Instant};

use sans_io_runtime::{
    collections::DynamicDeque, return_if_none, Buffer, BusChannelControl, Task, TaskSwitcherChild,
};
use str0m::{
    change::{DtlsCert, SdpOffer},
    ice::IceCreds,
    media::{KeyframeRequestKind, MediaKind, Mid},
    net::{Protocol, Receive},
    Candidate, Event as Str0mEvent, IceConnectionState, Input, Output, Rtc,
};

use super::{ChannelId, TrackMedia};

pub struct WhepTaskBuildResult {
    pub task: WhepTask,
    pub ice_ufrag: String,
    pub sdp: String,
}

pub enum WhepInput {
    UdpPacket {
        from: SocketAddr,
        data: Buffer,
    },
    Bus {
        channel: ChannelId,
        media: TrackMedia,
    },
}

pub enum WhepOutput {
    UdpPacket { to: SocketAddr, data: Buffer },
    Bus(BusChannelControl<ChannelId, KeyframeRequestKind>),
    Destroy,
}

pub struct WhepTask {
    backend_addr: SocketAddr,
    timeout: Option<Instant>,
    rtc: Rtc,
    audio_mid: Option<Mid>,
    video_mid: Option<Mid>,
    channel_id: u64,
    output: DynamicDeque<WhepOutput, 16>,
    has_input: bool,
}

impl WhepTask {
    pub fn build(
        backend_addr: SocketAddr,
        dtls_cert: DtlsCert,
        channel: u64,
        sdp: &str,
    ) -> Result<WhepTaskBuildResult, String> {
        let rtc_config = Rtc::builder()
            .set_rtp_mode(true)
            .set_ice_lite(true)
            .set_dtls_cert(dtls_cert)
            .set_local_ice_credentials(IceCreds::new());

        let ice_ufrag = rtc_config
            .local_ice_credentials()
            .as_ref()
            .expect("should have ice credentials")
            .ufrag
            .clone();

        let mut rtc = rtc_config.build();
        rtc.direct_api().enable_twcc_feedback();

        rtc.add_local_candidate(
            Candidate::host(backend_addr, Protocol::Udp).expect("Should create candidate"),
        );

        let offer = SdpOffer::from_sdp_string(&sdp).expect("Should parse offer");
        let answer = rtc
            .sdp_api()
            .accept_offer(offer)
            .expect("Should accept offer");
        let instance = Self {
            backend_addr,
            timeout: None,
            rtc,
            audio_mid: None,
            video_mid: None,
            channel_id: channel,
            output: DynamicDeque::default(),
            has_input: false,
        };

        Ok(WhepTaskBuildResult {
            task: instance,
            ice_ufrag,
            sdp: answer.to_sdp_string(),
        })
    }

    fn pop_event_inner(&mut self, now: Instant, has_input: bool) -> Option<WhepOutput> {
        if let Some(o) = self.output.pop_front() {
            return Some(o);
        }

        // incase we have input, we should not check timeout
        if !has_input {
            if let Some(timeout) = self.timeout {
                if timeout > now {
                    return None;
                }
            }
        }

        while let Ok(out) = self.rtc.poll_output() {
            match out {
                Output::Timeout(timeout) => {
                    self.timeout = Some(timeout);
                    return None;
                }
                Output::Transmit(send) => {
                    return Some(WhepOutput::UdpPacket {
                        to: send.destination,
                        data: Buffer::from(send.contents.to_vec()),
                    });
                }
                Output::Event(e) => match e {
                    Str0mEvent::Connected => {
                        log::info!("WhepServerTask connected");
                        self.output
                            .push_back(WhepOutput::Bus(BusChannelControl::Subscribe(
                                ChannelId::ConsumeAudio(self.channel_id),
                            )));
                        self.output
                            .push_back(WhepOutput::Bus(BusChannelControl::Subscribe(
                                ChannelId::ConsumeVideo(self.channel_id),
                            )));
                        self.output
                            .push_back(WhepOutput::Bus(BusChannelControl::Publish(
                                ChannelId::PublishVideo(self.channel_id),
                                true,
                                KeyframeRequestKind::Pli,
                            )));
                        return self.output.pop_front();
                    }
                    Str0mEvent::MediaAdded(media) => {
                        log::info!("WhepServerTask media added: {:?}", media);
                        if media.kind == MediaKind::Audio {
                            self.audio_mid = Some(media.mid);
                        } else {
                            self.video_mid = Some(media.mid);
                        }
                    }
                    Str0mEvent::IceConnectionStateChange(state) => match state {
                        IceConnectionState::Disconnected => {
                            self.output
                                .push_back(WhepOutput::Bus(BusChannelControl::Unsubscribe(
                                    ChannelId::ConsumeAudio(self.channel_id),
                                )));
                            self.output
                                .push_back(WhepOutput::Bus(BusChannelControl::Unsubscribe(
                                    ChannelId::ConsumeVideo(self.channel_id),
                                )));
                            self.output.push_back(WhepOutput::Destroy);
                            return self.output.pop_front();
                        }
                        _ => {}
                    },
                    Str0mEvent::KeyframeRequest(req) => {
                        return Some(WhepOutput::Bus(BusChannelControl::Publish(
                            ChannelId::PublishVideo(self.channel_id),
                            false,
                            req.kind,
                        )));
                    }
                    _ => {}
                },
            }
        }

        None
    }
}

impl Task<WhepInput, WhepOutput> for WhepTask {
    /// Called on each tick of the task.
    fn on_tick(&mut self, now: Instant) {
        let timeout = return_if_none!(self.timeout);
        if now < timeout {
            return;
        }

        self.has_input = true;
        if let Err(e) = self.rtc.handle_input(Input::Timeout(now)) {
            log::error!("Error handling timeout: {}", e);
        }
        log::trace!("clear timeout after handled");
        self.timeout = None;
    }

    /// Called when an input event is received for the task.
    fn on_event(&mut self, now: Instant, input: WhepInput) {
        match input {
            WhepInput::UdpPacket { from, data } => {
                self.has_input = true;
                if let Err(e) = self.rtc.handle_input(Input::Receive(
                    now,
                    Receive::new(Protocol::Udp, from, self.backend_addr, &data)
                        .expect("Should parse udp"),
                )) {
                    log::error!("Error handling udp: {}", e);
                }
                self.timeout = None;
            }
            WhepInput::Bus { channel, media } => {
                let (mid, nackable) = if matches!(channel, ChannelId::ConsumeAudio(..)) {
                    (self.audio_mid, false)
                } else {
                    (self.video_mid, true)
                };

                if let Some(mid) = mid {
                    self.has_input = true;
                    if let Some(stream) = self.rtc.direct_api().stream_tx_by_mid(mid, None) {
                        log::trace!(
                            "Write rtp for mid: {:?} {} {} {}",
                            mid,
                            media.header.payload_type,
                            media.header.sequence_number,
                            media.payload.len()
                        );
                        if let Err(e) = stream.write_rtp(
                            media.header.payload_type,
                            media.seq_no,
                            media.header.timestamp,
                            media.timestamp,
                            media.header.marker,
                            media.header.ext_vals,
                            nackable,
                            media.payload,
                        ) {
                            log::error!("Error writing rtp: {}", e);
                        }
                        log::trace!("clear timeout with media");
                        self.timeout = None;
                    }
                } else {
                    log::error!("No mid for media {}", media.header.payload_type);
                }
            }
        }
    }

    fn on_shutdown(&mut self, _now: Instant) {
        self.has_input = true;
        self.rtc.disconnect();
        self.output
            .push_back(WhepOutput::Bus(BusChannelControl::Unsubscribe(
                ChannelId::ConsumeAudio(self.channel_id),
            )));
        self.output
            .push_back(WhepOutput::Bus(BusChannelControl::Unsubscribe(
                ChannelId::ConsumeVideo(self.channel_id),
            )));
        self.output.push_back(WhepOutput::Destroy);
    }
}

impl TaskSwitcherChild<WhepOutput> for WhepTask {
    type Time = Instant;
    /// Retrieves the next output event from the task.
    fn pop_output(&mut self, now: Instant) -> Option<WhepOutput> {
        if self.has_input {
            self.has_input = false;
            self.pop_event_inner(now, true)
        } else {
            self.pop_event_inner(now, false)
        }
    }
}
