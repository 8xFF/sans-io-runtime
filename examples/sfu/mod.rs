use std::{
    collections::{hash_map::DefaultHasher, HashMap, VecDeque},
    hash::{Hash, Hasher},
    net::SocketAddr,
    time::Instant,
};

use derive_more::Display;
use sans_io_runtime::{
    backend::{BackendIncoming, BackendOutgoing},
    group_owner_type, group_task, TaskSwitcher, WorkerInner, WorkerInnerInput, WorkerInnerOutput,
};
use str0m::{
    change::DtlsCert,
    media::{KeyframeRequestKind, MediaTime},
    rtp::{RtpHeader, RtpPacket, SeqNo},
};

use crate::http::{HttpRequest, HttpResponse};

use self::{
    shared_port::SharedUdpPort,
    whep::{WhepInput, WhepOutput, WhepTask},
    whip::{WhipInput, WhipOutput, WhipTask},
};

mod shared_port;
mod whep;
mod whip;

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
pub enum TaskId {
    Whip(usize),
    Whep(usize),
}

#[derive(Debug, Clone)]
pub enum ExtIn {}

#[derive(Debug, Clone)]
pub enum ExtOut {
    HttpResponse(HttpResponse),
}

#[derive(Display, Debug, Clone, Copy, Hash, PartialEq, Eq)]
pub enum ChannelId {
    ConsumeAudio(u64),
    ConsumeVideo(u64),
    PublishVideo(u64),
}

#[derive(Debug, Clone)]
pub struct TrackMedia {
    /// Extended sequence number to avoid having to deal with ROC.
    pub seq_no: SeqNo,

    /// Extended RTP time in the clock frequency of the codec. To avoid dealing with ROC.
    ///
    /// For a newly scheduled outgoing packet, the clock_rate is not correctly set until
    /// we do the poll_output().
    pub time: MediaTime,

    /// Parsed RTP header.
    pub header: RtpHeader,

    /// RTP payload. This contains no header.
    pub payload: Vec<u8>,

    /// str0m server timestamp.
    ///
    /// This timestamp has nothing to do with RTP itself. For outgoing packets, this is when
    /// the packet was first handed over to str0m and enqueued in the outgoing send buffers.
    /// For incoming packets it's the time we received the network packet.
    pub timestamp: Instant,
}

#[derive(Clone, Debug, convert_enum::From)]
pub enum SfuEvent {
    RequestKeyFrame(KeyframeRequestKind),
    Media(TrackMedia),
}

#[derive(Debug, Clone)]
pub struct ICfg {
    pub udp_addr: SocketAddr,
}

#[derive(Debug, Clone)]
pub enum SCfg {
    HttpRequest(HttpRequest),
}

group_owner_type!(WhipOwner);
group_task!(WhipTaskGroup, WhipTask, WhipInput<'a>, WhipOutput);

group_owner_type!(WhepOwner);
group_task!(WhepTaskGroup, WhepTask, WhepInput<'a>, WhepOutput);

#[derive(convert_enum::From, Debug, Clone, Copy, PartialEq)]
pub enum OwnerType {
    Whip(WhipOwner),
    Whep(WhepOwner),
    #[convert_enum(optout)]
    System,
}

pub struct SfuWorker {
    worker: u16,
    dtls_cert: DtlsCert,
    whip_group: WhipTaskGroup,
    whep_group: WhepTaskGroup,
    output: VecDeque<WorkerInnerOutput<'static, OwnerType, ExtOut, ChannelId, SfuEvent, SCfg>>,
    shared_udp: SharedUdpPort<TaskId>,
    switcher: TaskSwitcher,
}

impl SfuWorker {
    fn channel_build(channel: &str) -> u64 {
        let mut hasher = DefaultHasher::new();
        channel.hash(&mut hasher);
        hasher.finish()
    }

    fn process_req(&mut self, req: HttpRequest) {
        match req.path.as_str() {
            "/whip/endpoint" => self.connect_whip(req),
            "/whep/endpoint" => self.connect_whep(req),
            _ => {
                self.output.push_back(WorkerInnerOutput::Ext(
                    true,
                    ExtOut::HttpResponse(HttpResponse {
                        req_id: req.req_id,
                        status: 404,
                        headers: HashMap::new(),
                        body: b"Task Not Found".to_vec(),
                    }),
                ));
            }
        }
    }

    fn connect_whip(&mut self, req: HttpRequest) {
        let http_auth = req.http_auth();
        log::info!("Whip endpoint connect request: {}", http_auth);
        let channel = Self::channel_build(&http_auth);
        let task = WhipTask::build(
            self.shared_udp.get_backend_addr().expect(""),
            self.dtls_cert.clone(),
            channel,
            &String::from_utf8_lossy(&req.body),
        );
        match task {
            Ok(task) => {
                log::info!("Whip endpoint created {}", task.ice_ufrag);
                let index = self.whip_group.add_task(task.task);
                self.shared_udp
                    .add_ufrag(task.ice_ufrag, TaskId::Whip(index));
                self.output.push_back(WorkerInnerOutput::Ext(
                    true,
                    ExtOut::HttpResponse(HttpResponse {
                        req_id: req.req_id,
                        status: 200,
                        headers: HashMap::from([
                            ("Content-Type".to_string(), "application/sdp".to_string()),
                            (
                                "Location".to_string(),
                                format!("/whip/endpoint/{}/{index}", self.worker),
                            ),
                        ]),
                        body: task.sdp.into_bytes(),
                    }),
                ));
            }
            Err(err) => {
                log::error!("Error creating whip endpoint: {}", err);
                self.output.push_back(WorkerInnerOutput::Ext(
                    true,
                    ExtOut::HttpResponse(HttpResponse {
                        req_id: req.req_id,
                        status: 500,
                        headers: HashMap::new(),
                        body: err.into_bytes(),
                    }),
                ));
            }
        }
    }

    fn connect_whep(&mut self, req: HttpRequest) {
        let http_auth = req.http_auth();
        log::info!("Whep endpoint connect request: {}", http_auth);
        let channel = Self::channel_build(&http_auth);
        let task = WhepTask::build(
            self.shared_udp.get_backend_addr().expect(""),
            self.dtls_cert.clone(),
            channel,
            &String::from_utf8_lossy(&req.body),
        );
        match task {
            Ok(task) => {
                log::info!("Whep endpoint created {}", task.ice_ufrag);
                let index = self.whep_group.add_task(task.task);
                self.shared_udp
                    .add_ufrag(task.ice_ufrag, TaskId::Whep(index));
                self.output.push_back(WorkerInnerOutput::Ext(
                    true,
                    ExtOut::HttpResponse(HttpResponse {
                        req_id: req.req_id,
                        status: 200,
                        headers: HashMap::from([
                            ("Content-Type".to_string(), "application/sdp".to_string()),
                            (
                                "Location".to_string(),
                                format!("/whep/endpoint/{}/{index}", self.worker),
                            ),
                        ]),
                        body: task.sdp.into_bytes(),
                    }),
                ));
            }
            Err(err) => {
                log::error!("Error creating whep endpoint: {}", err);
                self.output.push_back(WorkerInnerOutput::Ext(
                    true,
                    ExtOut::HttpResponse(HttpResponse {
                        req_id: req.req_id,
                        status: 500,
                        headers: HashMap::new(),
                        body: err.into_bytes(),
                    }),
                ));
            }
        }
    }
}

#[repr(u8)]
enum TaskType {
    Whip = 0,
    Whep = 1,
}

impl From<usize> for TaskType {
    fn from(value: usize) -> Self {
        match value {
            0 => Self::Whip,
            1 => Self::Whep,
            _ => panic!("Should not happend"),
        }
    }
}

impl SfuWorker {
    fn process_whip_out<'a>(
        &mut self,
        _now: Instant,
        index: usize,
        out: WhipOutput,
    ) -> WorkerInnerOutput<'a, OwnerType, ExtOut, ChannelId, SfuEvent, SCfg> {
        self.switcher.queue_flag_task(TaskType::Whip as usize);
        let owner = OwnerType::Whip(index.into());
        match out {
            WhipOutput::Bus(control) => WorkerInnerOutput::Bus(owner, control.convert_into()),
            WhipOutput::UdpPacket { to, data } => WorkerInnerOutput::Net(
                owner,
                BackendOutgoing::UdpPacket {
                    slot: self.shared_udp.get_backend_slot().expect(""),
                    to,
                    data,
                },
            ),
            WhipOutput::Destroy => {
                self.shared_udp.remove_task(TaskId::Whip(index));
                self.whip_group.remove_task(index);
                log::info!(
                    "destroy whip({index}) => remain {}",
                    self.whip_group.tasks()
                );
                WorkerInnerOutput::Destroy(owner)
            }
        }
    }

    fn process_whep_out<'a>(
        &mut self,
        _now: Instant,
        index: usize,
        out: WhepOutput,
    ) -> WorkerInnerOutput<'a, OwnerType, ExtOut, ChannelId, SfuEvent, SCfg> {
        self.switcher.queue_flag_task(TaskType::Whep as usize);
        let owner = OwnerType::Whep(index.into());
        match out {
            WhepOutput::Bus(control) => WorkerInnerOutput::Bus(owner, control.convert_into()),
            WhepOutput::UdpPacket { to, data } => WorkerInnerOutput::Net(
                owner,
                BackendOutgoing::UdpPacket {
                    slot: self.shared_udp.get_backend_slot().expect(""),
                    to,
                    data,
                },
            ),
            WhepOutput::Destroy => {
                self.shared_udp.remove_task(TaskId::Whip(index));
                self.whep_group.remove_task(index);
                log::info!(
                    "destroy whep({index}) => remain {}",
                    self.whep_group.tasks()
                );
                WorkerInnerOutput::Destroy(owner)
            }
        }
    }
}

impl WorkerInner<OwnerType, ExtIn, ExtOut, ChannelId, SfuEvent, ICfg, SCfg> for SfuWorker {
    fn build(worker: u16, cfg: ICfg) -> Self {
        Self {
            worker,
            dtls_cert: DtlsCert::new_openssl(),
            whip_group: WhipTaskGroup::default(),
            whep_group: WhepTaskGroup::default(),
            output: VecDeque::from([WorkerInnerOutput::Net(
                OwnerType::System,
                BackendOutgoing::UdpListen {
                    addr: cfg.udp_addr,
                    reuse: false,
                },
            )]),
            shared_udp: SharedUdpPort::default(),
            switcher: TaskSwitcher::new(2),
        }
    }
    fn worker_index(&self) -> u16 {
        self.worker
    }
    fn tasks(&self) -> usize {
        self.whip_group.tasks() + self.whep_group.tasks()
    }
    fn spawn(&mut self, _now: Instant, cfg: SCfg) {
        match cfg {
            SCfg::HttpRequest(req) => {
                self.process_req(req);
            }
        }
    }
    fn on_tick<'a>(
        &mut self,
        now: Instant,
    ) -> Option<WorkerInnerOutput<'a, OwnerType, ExtOut, ChannelId, SfuEvent, SCfg>> {
        if let Some(e) = self.output.pop_front() {
            return Some(e.into());
        }

        let switcher = &mut self.switcher;
        loop {
            match switcher.looper_current(now)?.into() {
                TaskType::Whip => {
                    if let Some((index, out)) =
                        switcher.looper_process(self.whip_group.on_tick(now))
                    {
                        return Some(self.process_whip_out(now, index, out));
                    }
                }
                TaskType::Whep => {
                    if let Some((index, out)) =
                        switcher.looper_process(self.whep_group.on_tick(now))
                    {
                        return Some(self.process_whep_out(now, index, out));
                    }
                }
            }
        }
    }

    fn on_event<'a>(
        &mut self,
        now: Instant,
        event: WorkerInnerInput<'a, OwnerType, ExtIn, ChannelId, SfuEvent>,
    ) -> Option<WorkerInnerOutput<'a, OwnerType, ExtOut, ChannelId, SfuEvent, SCfg>> {
        match event {
            WorkerInnerInput::Net(_owner, BackendIncoming::UdpListenResult { bind: _, result }) => {
                log::info!("UdpListenResult: {:?}", result);
                let (addr, slot) = result.as_ref().expect("Should listen shared port ok");
                self.shared_udp.set_backend_info(*addr, *slot);
                None
            }
            WorkerInnerInput::Net(
                _owner,
                BackendIncoming::UdpPacket {
                    from,
                    slot: _,
                    data,
                },
            ) => match self.shared_udp.map_remote(from, &data) {
                Some(TaskId::Whip(index)) => {
                    let out = self.whip_group.on_event(
                        now,
                        index,
                        WhipInput::UdpPacket {
                            from,
                            data: data.freeze(),
                        },
                    )?;
                    Some(self.process_whip_out(now, index, out))
                }
                Some(TaskId::Whep(index)) => {
                    let out = self.whep_group.on_event(
                        now,
                        index,
                        WhepInput::UdpPacket {
                            from,
                            data: data.freeze(),
                        },
                    )?;
                    Some(self.process_whep_out(now, index, out))
                }
                None => {
                    log::debug!("Unknown remote address: {}", from);
                    None
                }
            },
            WorkerInnerInput::Bus(owner, channel, event) => match (owner, event) {
                (OwnerType::Whip(owner), SfuEvent::RequestKeyFrame(kind)) => {
                    let out = self.whip_group.on_event(
                        now,
                        owner.index(),
                        WhipInput::Bus { channel, kind },
                    )?;
                    Some(self.process_whip_out(now, owner.index(), out))
                }
                (OwnerType::Whep(owner), SfuEvent::Media(media)) => {
                    let out = self.whep_group.on_event(
                        now,
                        owner.index(),
                        WhepInput::Bus { channel, media },
                    )?;
                    Some(self.process_whep_out(now, owner.index(), out))
                }
                _ => panic!("should not hapend {:?}", owner),
            },
            _ => None,
        }
    }

    fn pop_output<'a>(
        &mut self,
        now: Instant,
    ) -> Option<WorkerInnerOutput<'a, OwnerType, ExtOut, ChannelId, SfuEvent, SCfg>> {
        let switcher = &mut self.switcher;
        while let Some(current) = switcher.queue_current() {
            match current.into() {
                TaskType::Whip => {
                    if let Some((index, out)) =
                        switcher.queue_process(self.whip_group.pop_output(now))
                    {
                        return Some(self.process_whip_out(now, index, out));
                    }
                }
                TaskType::Whep => {
                    if let Some((index, out)) =
                        switcher.queue_process(self.whep_group.pop_output(now))
                    {
                        return Some(self.process_whep_out(now, index, out));
                    }
                }
            }
        }
        None
    }

    fn shutdown<'a>(
        &mut self,
        now: Instant,
    ) -> Option<WorkerInnerOutput<'a, OwnerType, ExtOut, ChannelId, SfuEvent, SCfg>> {
        let switcher = &mut self.switcher;
        loop {
            match switcher.looper_current(now)?.into() {
                TaskType::Whip => {
                    if let Some((index, out)) =
                        switcher.looper_process(self.whip_group.shutdown(now))
                    {
                        return Some(self.process_whip_out(now, index, out));
                    }
                }
                TaskType::Whep => {
                    if let Some((index, out)) =
                        switcher.looper_process(self.whep_group.shutdown(now))
                    {
                        return Some(self.process_whep_out(now, index, out));
                    }
                }
            }
        }
    }
}

impl TrackMedia {
    pub fn from_raw(rtp: RtpPacket) -> Self {
        let header = rtp.header;
        let payload = rtp.payload;
        let time = rtp.time;
        let timestamp = rtp.timestamp;
        let seq_no = rtp.seq_no;

        Self {
            seq_no,
            time,
            header,
            payload,
            timestamp,
        }
    }
}
