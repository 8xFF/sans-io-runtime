use std::{net::SocketAddr, time::Instant};

use sans_io_runtime::{
    bus::BusEvent, collections::DynamicDeque, Buffer, NetIncoming, NetOutgoing, Task, TaskInput,
    TaskOutput,
};
use str0m::{
    change::{DtlsCert, SdpOffer},
    media::{KeyframeRequestKind, MediaKind, Mid},
    net::{Protocol, Receive},
    Candidate, Event, IceConnectionState, Input, Output, Rtc,
};

use super::{ChannelId, SfuEvent, TrackMedia};

pub struct WhepTaskBuildResult {
    pub task: WhepTask,
    pub ice_ufrag: String,
    pub sdp: String,
}

pub struct WhepTask {
    timeout: Option<Instant>,
    rtc: Rtc,
    audio_mid: Option<Mid>,
    video_mid: Option<Mid>,
    channel_id: u64,
    output: DynamicDeque<TaskOutput<'static, ChannelId, SfuEvent>, 16>,
}

impl WhepTask {
    pub fn build(
        addr: SocketAddr,
        dtls_cert: DtlsCert,
        channel: u64,
        sdp: &str,
    ) -> Result<WhepTaskBuildResult, String> {
        let rtc_config = Rtc::builder()
            .set_rtp_mode(true)
            .set_ice_lite(true)
            .set_dtls_cert(dtls_cert);
        let ice_ufrag = rtc_config.local_ice_credentials().ufrag.clone();
        let mut rtc = rtc_config.build();
        rtc.direct_api().enable_twcc_feedback();

        rtc.add_local_candidate(
            Candidate::host(addr, Protocol::Udp).expect("Should create candidate"),
        );

        let offer = SdpOffer::from_sdp_string(&sdp).expect("Should parse offer");
        let answer = rtc
            .sdp_api()
            .accept_offer(offer)
            .expect("Should accept offer");
        let instance = Self {
            timeout: None,
            rtc,
            audio_mid: None,
            video_mid: None,
            channel_id: channel,
            output: DynamicDeque::new(),
        };

        Ok(WhepTaskBuildResult {
            task: instance,
            ice_ufrag,
            sdp: answer.to_sdp_string(),
        })
    }
}

impl Task<ChannelId, SfuEvent> for WhepTask {
    /// The type identifier for the task.
    const TYPE: u16 = 2;

    /// Called on each tick of the task.
    fn on_tick(&mut self, now: Instant) {
        if let Some(timeout) = self.timeout {
            if now >= timeout {
                if let Err(e) = self.rtc.handle_input(Input::Timeout(now)) {
                    log::error!("Error handling timeout: {}", e);
                }
                log::trace!("clear timeout after handled");
                self.timeout = None;
            }
        }
    }

    /// Called when an input event is received for the task.
    fn on_input<'a>(&mut self, now: Instant, input: TaskInput<'a, ChannelId, SfuEvent>) {
        match input {
            TaskInput::Net(event) => match event {
                NetIncoming::UdpPacket { from, to, data } => {
                    if let Err(e) = self.rtc.handle_input(Input::Receive(
                        now,
                        Receive::new(Protocol::Udp, from, to, data).expect("Should parse udp"),
                    )) {
                        log::error!("Error handling udp: {}", e);
                    }
                    self.timeout = None;
                }
                NetIncoming::UdpListenResult { .. } => {
                    panic!("Unexpected UdpListenResult");
                }
            },
            TaskInput::Bus(channel, event) => match event {
                SfuEvent::RequestKeyFrame(_kind) => {
                    log::warn!("RequestKeyFrame event should not be sent to WhepTask");
                }
                SfuEvent::Media(media) => {
                    let (mid, nackable) = if matches!(channel, ChannelId::ConsumeAudio(..)) {
                        (self.audio_mid, false)
                    } else {
                        (self.video_mid, true)
                    };

                    if let Some(mid) = mid {
                        if let Some(stream) = self.rtc.direct_api().stream_tx_by_mid(mid, None) {
                            log::debug!(
                                "Write rtp for mid: {:?} {} {} {}",
                                mid,
                                media.seq_no,
                                media.header.timestamp,
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
            },
        }
    }

    /// Retrieves the next output event from the task.
    fn pop_output(&mut self, now: Instant) -> Option<TaskOutput<'_, ChannelId, SfuEvent>> {
        if let Some(o) = self.output.pop_front() {
            return Some(o);
        }

        if let Some(timeout) = self.timeout {
            if timeout > now {
                return None;
            }
        }

        match self.rtc.poll_output().ok()? {
            Output::Timeout(timeout) => {
                self.timeout = Some(timeout);
                None
            }
            Output::Transmit(send) => Some(
                TaskOutput::Net(NetOutgoing::UdpPacket {
                    from: send.source,
                    to: send.destination,
                    data: Buffer::Vec(send.contents.into()),
                })
                .into(),
            ),
            Output::Event(e) => match e {
                Event::Connected => {
                    log::info!("WhepServerTask connected");
                    self.output
                        .push_back_safe(TaskOutput::Bus(BusEvent::ChannelSubscribe(
                            ChannelId::ConsumeAudio(self.channel_id),
                        )));
                    self.output
                        .push_back_safe(TaskOutput::Bus(BusEvent::ChannelSubscribe(
                            ChannelId::ConsumeVideo(self.channel_id),
                        )));
                    self.output
                        .push_back_safe(TaskOutput::Bus(BusEvent::ChannelPublish(
                            ChannelId::PublishVideo(self.channel_id),
                            true,
                            SfuEvent::RequestKeyFrame(KeyframeRequestKind::Pli),
                        )));
                    None
                }
                Event::MediaAdded(media) => {
                    log::info!("WhepServerTask media added: {:?}", media);
                    if media.kind == MediaKind::Audio {
                        self.audio_mid = Some(media.mid);
                    } else {
                        self.video_mid = Some(media.mid);
                    }
                    None
                }
                Event::IceConnectionStateChange(state) => match state {
                    IceConnectionState::Disconnected => {
                        self.output
                            .push_back_safe(TaskOutput::Bus(BusEvent::ChannelUnsubscribe(
                                ChannelId::ConsumeAudio(self.channel_id),
                            )));
                        self.output
                            .push_back_safe(TaskOutput::Bus(BusEvent::ChannelUnsubscribe(
                                ChannelId::ConsumeVideo(self.channel_id),
                            )));
                        self.output.push_back_safe(TaskOutput::Destroy);
                        self.output.pop_front()
                    }
                    _ => None,
                },
                Event::RtpPacket(rtp) => {
                    let channel = if *rtp.header.payload_type == 111 {
                        ChannelId::ConsumeAudio(self.channel_id)
                    } else {
                        ChannelId::ConsumeVideo(self.channel_id)
                    };
                    let media = TrackMedia::from_raw(rtp);
                    log::info!(
                        "publish to channel {:?}, size {}",
                        channel,
                        media.payload.len()
                    );
                    Some(TaskOutput::Bus(BusEvent::ChannelPublish(
                        channel,
                        false,
                        SfuEvent::Media(media),
                    )))
                }
                _ => None,
            },
        }
    }
}
