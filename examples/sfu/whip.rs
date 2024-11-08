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

pub struct WhipTaskBuildResult {
    pub task: WhipTask,
    pub ice_ufrag: String,
    pub sdp: String,
}

pub enum WhipInput {
    UdpPacket { from: SocketAddr, data: Buffer },
    Bus { kind: KeyframeRequestKind },
}

pub enum WhipOutput {
    UdpPacket { to: SocketAddr, data: Buffer },
    Bus(BusChannelControl<ChannelId, TrackMedia>),
    OnResourceEmpty,
}

pub struct WhipTask {
    backend_addr: SocketAddr,
    timeout: Option<Instant>,
    rtc: Rtc,
    audio_mid: Option<Mid>,
    video_mid: Option<Mid>,
    channel_id: u64,
    output: DynamicDeque<WhipOutput, 8>,
    has_input: bool,
    disconnected: bool,
}

impl WhipTask {
    pub fn build(
        backend_addr: SocketAddr,
        dtls_cert: DtlsCert,
        channel: u64,
        sdp: &str,
    ) -> Result<WhipTaskBuildResult, String> {
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
            disconnected: false,
        };

        Ok(WhipTaskBuildResult {
            task: instance,
            ice_ufrag,
            sdp: answer.to_sdp_string(),
        })
    }

    fn pop_event_inner(&mut self, now: Instant, has_input: bool) -> Option<WhipOutput> {
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
                    break;
                }
                Output::Transmit(send) => {
                    return WhipOutput::UdpPacket {
                        to: send.destination,
                        data: Buffer::from(send.contents.to_vec()),
                    }
                    .into();
                }
                Output::Event(e) => match e {
                    Str0mEvent::Connected => {
                        log::info!("WhipServerTask connected");
                        return WhipOutput::Bus(BusChannelControl::Subscribe(
                            ChannelId::PublishVideo(self.channel_id),
                        ))
                        .into();
                    }
                    Str0mEvent::MediaAdded(media) => {
                        log::info!("WhipServerTask media added: {:?}", media);
                        if media.kind == MediaKind::Audio {
                            self.audio_mid = Some(media.mid);
                        } else {
                            self.video_mid = Some(media.mid);
                        }
                    }
                    Str0mEvent::IceConnectionStateChange(state) => match state {
                        IceConnectionState::Disconnected => {
                            log::info!("WhipServerTask disconnected");
                            self.output
                                .push_back(WhipOutput::Bus(BusChannelControl::Unsubscribe(
                                    ChannelId::PublishVideo(self.channel_id),
                                )));
                            self.disconnected = true;
                            return self.output.pop_front();
                        }
                        _ => {}
                    },
                    Str0mEvent::RtpPacket(rtp) => {
                        let channel = if *rtp.header.payload_type == 111 {
                            ChannelId::ConsumeAudio(self.channel_id)
                        } else {
                            ChannelId::ConsumeVideo(self.channel_id)
                        };
                        let media = TrackMedia::from_raw(rtp);
                        log::trace!(
                            "WhipServerTask on video {} {} {}",
                            media.header.payload_type,
                            media.header.sequence_number,
                            media.payload.len(),
                        );
                        return Some(WhipOutput::Bus(BusChannelControl::Publish(
                            channel, false, media,
                        )));
                    }
                    _ => {}
                },
            }
        }

        None
    }
}

impl Task<WhipInput, WhipOutput> for WhipTask {
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
        self.timeout = None;
    }

    /// Called when an input event is received for the task.
    fn on_event(&mut self, now: Instant, input: WhipInput) {
        match input {
            WhipInput::UdpPacket { from, data } => {
                self.has_input = true;
                if let Err(e) = self.rtc.handle_input(Input::Receive(
                    now,
                    Receive::new(Protocol::Udp, from, self.backend_addr, &data)
                        .expect("Should parse udp"),
                )) {
                    log::error!("Error handling udp: {}", e);
                }
            }
            WhipInput::Bus { kind } => {
                if let Some(mid) = self.video_mid {
                    log::info!("Requesting keyframe for video mid: {:?}", mid);
                    self.has_input = true;
                    self.rtc
                        .direct_api()
                        .stream_rx_by_mid(mid, None)
                        .expect("Should has video mid")
                        .request_keyframe(kind);
                } else {
                    log::error!("No video mid for requesting keyframe");
                }
            }
        }
    }

    fn on_shutdown(&mut self, _now: Instant) {
        log::info!("WhipServerTask on shutdown");
        self.has_input = true;
        self.rtc.disconnect();
        self.output
            .push_back(WhipOutput::Bus(BusChannelControl::Unsubscribe(
                ChannelId::PublishVideo(self.channel_id),
            )));
        self.disconnected = true;
    }
}

impl TaskSwitcherChild<WhipOutput> for WhipTask {
    type Time = Instant;

    fn empty_event(&self) -> WhipOutput {
        WhipOutput::OnResourceEmpty
    }

    fn is_empty(&self) -> bool {
        self.disconnected && self.output.is_empty()
    }

    /// Retrieves the next output event from the task.
    fn pop_output(&mut self, now: Instant) -> Option<WhipOutput> {
        if self.has_input {
            self.has_input = false;
            self.pop_event_inner(now, true)
        } else {
            self.pop_event_inner(now, false)
        }
    }
}

impl Drop for WhipTask {
    fn drop(&mut self) {
        log::info!("WhipServerTask dropped");
        assert_eq!(
            self.output.len(),
            0,
            "WhipTask should be empty when dropped"
        );
    }
}
