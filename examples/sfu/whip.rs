use std::{net::SocketAddr, time::Instant};

use sans_io_runtime::{
    bus::BusEvent, collections::DynamicDeque, Buffer, NetIncoming, NetOutgoing, Task, TaskInput,
    TaskOutput,
};
use str0m::{
    change::{DtlsCert, SdpOffer},
    media::{MediaKind, Mid},
    net::{Protocol, Receive},
    Candidate, Event, IceConnectionState, Input, Output, Rtc,
};

use super::{ChannelId, SfuEvent, TrackMedia};

pub struct WhipTaskBuildResult {
    pub task: WhipTask,
    pub ice_ufrag: String,
    pub sdp: String,
}

pub struct WhipTask {
    timeout: Option<Instant>,
    rtc: Rtc,
    audio_mid: Option<Mid>,
    video_mid: Option<Mid>,
    channel_id: u64,
    output: DynamicDeque<TaskOutput<'static, ChannelId, SfuEvent>, 8>,
}

impl WhipTask {
    pub fn build(
        addr: SocketAddr,
        dtls_cert: DtlsCert,
        channel: u64,
        sdp: &str,
    ) -> Result<WhipTaskBuildResult, String> {
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

        Ok(WhipTaskBuildResult {
            task: instance,
            ice_ufrag,
            sdp: answer.to_sdp_string(),
        })
    }

    fn pop_event_inner(
        &mut self,
        now: Instant,
        has_input: bool,
    ) -> Option<TaskOutput<'static, ChannelId, SfuEvent>> {
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
                    return TaskOutput::Net(NetOutgoing::UdpPacket {
                        from: send.source,
                        to: send.destination,
                        data: Buffer::Vec(send.contents.into()),
                    })
                    .into();
                }
                Output::Event(e) => match e {
                    Event::Connected => {
                        log::info!("WhipServerTask connected");
                        return TaskOutput::Bus(BusEvent::ChannelSubscribe(
                            ChannelId::PublishVideo(self.channel_id),
                        ))
                        .into();
                    }
                    Event::MediaAdded(media) => {
                        log::info!("WhipServerTask media added: {:?}", media);
                        if media.kind == MediaKind::Audio {
                            self.audio_mid = Some(media.mid);
                        } else {
                            self.video_mid = Some(media.mid);
                        }
                    }
                    Event::IceConnectionStateChange(state) => match state {
                        IceConnectionState::Disconnected => {
                            self.output.push_back_safe(TaskOutput::Bus(
                                BusEvent::ChannelUnsubscribe(ChannelId::PublishVideo(
                                    self.channel_id,
                                )),
                            ));
                            self.output.push_back_safe(TaskOutput::Destroy);
                            return self.output.pop_front();
                        }
                        _ => {}
                    },
                    Event::RtpPacket(rtp) => {
                        let channel = if *rtp.header.payload_type == 111 {
                            ChannelId::ConsumeAudio(self.channel_id)
                        } else {
                            ChannelId::ConsumeVideo(self.channel_id)
                        };
                        let media = TrackMedia::from_raw(rtp);
                        return Some(TaskOutput::Bus(BusEvent::ChannelPublish(
                            channel,
                            false,
                            SfuEvent::Media(media),
                        )));
                    }
                    _ => {}
                },
            }
        }

        None
    }
}

impl Task<ChannelId, SfuEvent> for WhipTask {
    /// The type identifier for the task.
    const TYPE: u16 = 0;

    /// Called on each tick of the task.
    fn on_tick<'a>(&mut self, now: Instant) -> Option<TaskOutput<'a, ChannelId, SfuEvent>> {
        let timeout = self.timeout?;
        if now < timeout {
            return None;
        }

        if let Err(e) = self.rtc.handle_input(Input::Timeout(now)) {
            log::error!("Error handling timeout: {}", e);
        }
        self.timeout = None;
        self.pop_event_inner(now, true)
    }

    /// Called when an input event is received for the task.
    fn on_input<'a>(
        &mut self,
        now: Instant,
        input: TaskInput<'a, ChannelId, SfuEvent>,
    ) -> Option<TaskOutput<'a, ChannelId, SfuEvent>> {
        match input {
            TaskInput::Net(event) => match event {
                NetIncoming::UdpPacket { from, to, data } => {
                    if let Err(e) = self.rtc.handle_input(Input::Receive(
                        now,
                        Receive::new(Protocol::Udp, from, to, data).expect("Should parse udp"),
                    )) {
                        log::error!("Error handling udp: {}", e);
                    }
                    self.pop_event_inner(now, true)
                }
                NetIncoming::UdpListenResult { .. } => {
                    panic!("Unexpected UdpListenResult");
                }
            },
            TaskInput::Bus(channel, event) => match event {
                SfuEvent::RequestKeyFrame(kind) => {
                    if let Some(mid) = self.video_mid {
                        log::info!("Requesting keyframe for video mid: {:?}", mid);
                        self.rtc
                            .direct_api()
                            .stream_rx_by_mid(mid, None)
                            .expect("Should has video mid")
                            .request_keyframe(kind);
                        self.pop_event_inner(now, true)
                    } else {
                        log::error!("No video mid for requesting keyframe");
                        None
                    }
                }
                SfuEvent::Media(_media) => {
                    log::warn!(
                        "Media event should not be sent to WhipTask in {:?}",
                        channel
                    );
                    None
                }
            },
        }
    }

    /// Retrieves the next output event from the task.
    fn pop_output<'a>(&mut self, now: Instant) -> Option<TaskOutput<'a, ChannelId, SfuEvent>> {
        self.pop_event_inner(now, false)
    }
}
