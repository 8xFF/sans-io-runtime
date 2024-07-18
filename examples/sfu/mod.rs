use std::{
    collections::{hash_map::DefaultHasher, HashMap, VecDeque},
    hash::{Hash, Hasher},
    net::SocketAddr,
    time::Instant,
};

use derive_more::Display;
use sans_io_runtime::{
    backend::{BackendIncoming, BackendOutgoing},
    group_owner_type, return_if_some, BusControl, BusEvent, TaskGroup, TaskSwitcher,
    TaskSwitcherBranch, WorkerInner, WorkerInnerInput, WorkerInnerOutput,
};
use str0m::{
    change::DtlsCert,
    media::KeyframeRequestKind,
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
group_owner_type!(WhepOwner);

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
    whip_group:
        TaskSwitcherBranch<TaskGroup<WhipInput, WhipOutput, WhipTask, 64>, (usize, WhipOutput)>,
    whep_group:
        TaskSwitcherBranch<TaskGroup<WhepInput, WhepOutput, WhepTask, 64>, (usize, WhepOutput)>,
    shared_udp: SharedUdpPort<TaskId>,
    output: VecDeque<WorkerInnerOutput<OwnerType, ExtOut, ChannelId, SfuEvent, SCfg>>,
    switcher: TaskSwitcher,
}

impl SfuWorker {
    fn channel_build(channel: &str) -> u64 {
        let mut hasher = DefaultHasher::new();
        channel.hash(&mut hasher);
        hasher.finish()
    }

    fn process_req(&mut self, req: HttpRequest) {
        match (req.method.as_str(), req.path.as_str()) {
            ("POST", "/whip/endpoint") => self.connect_whip(req),
            ("POST", "/whep/endpoint") => self.connect_whep(req),
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
                let index = self
                    .whip_group
                    .input(&mut self.switcher)
                    .add_task(task.task);
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
                let index = self
                    .whep_group
                    .input(&mut self.switcher)
                    .add_task(task.task);
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

#[derive(num_enum::TryFromPrimitive, num_enum::IntoPrimitive)]
#[repr(usize)]
enum TaskType {
    Whip = 0,
    Whep = 1,
}

impl SfuWorker {
    fn process_whip_out(
        &mut self,
        _now: Instant,
        index: usize,
        out: WhipOutput,
    ) -> WorkerInnerOutput<OwnerType, ExtOut, ChannelId, SfuEvent, SCfg> {
        self.switcher.flag_task(TaskType::Whip as usize);
        let owner = OwnerType::Whip(index.into());
        match out {
            WhipOutput::Bus(control) => {
                WorkerInnerOutput::Bus(BusControl::Channel(owner, control.convert_into()))
            }
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
                let whip_group = self.whip_group.input(&mut self.switcher);
                whip_group.remove_task(index);
                log::info!("destroy whip({index}) => remain {}", whip_group.tasks());
                WorkerInnerOutput::Destroy(owner)
            }
        }
    }

    fn process_whep_out(
        &mut self,
        _now: Instant,
        index: usize,
        out: WhepOutput,
    ) -> WorkerInnerOutput<OwnerType, ExtOut, ChannelId, SfuEvent, SCfg> {
        self.switcher.flag_task(TaskType::Whep as usize);
        let owner = OwnerType::Whep(index.into());
        match out {
            WhepOutput::Bus(control) => {
                WorkerInnerOutput::Bus(BusControl::Channel(owner, control.convert_into()))
            }
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
                let whep_group = self.whep_group.input(&mut self.switcher);
                whep_group.remove_task(index);
                log::info!("destroy whep({index}) => remain {}", whep_group.tasks());
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
            whip_group: TaskSwitcherBranch::new(TaskGroup::default(), 0 as usize),
            whep_group: TaskSwitcherBranch::new(TaskGroup::default(), 1 as usize),
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
    fn on_tick(&mut self, now: Instant) {
        self.whip_group.input(&mut self.switcher).on_tick(now);
        self.whep_group.input(&mut self.switcher).on_tick(now);
    }

    fn on_event(
        &mut self,
        now: Instant,
        event: WorkerInnerInput<OwnerType, ExtIn, ChannelId, SfuEvent>,
    ) {
        match event {
            WorkerInnerInput::Net(_owner, BackendIncoming::UdpListenResult { bind: _, result }) => {
                log::info!("UdpListenResult: {:?}", result);
                let (addr, slot) = result.as_ref().expect("Should listen shared port ok");
                self.shared_udp.set_backend_info(*addr, *slot);
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
                    self.switcher.flag_task(TaskType::Whip);
                    self.whip_group.input(&mut self.switcher).on_event(
                        now,
                        index,
                        WhipInput::UdpPacket { from, data },
                    );
                }
                Some(TaskId::Whep(index)) => {
                    self.switcher.flag_task(TaskType::Whep);
                    self.whep_group.input(&mut self.switcher).on_event(
                        now,
                        index,
                        WhepInput::UdpPacket { from, data },
                    );
                }
                None => {
                    log::debug!("Unknown remote address: {}", from);
                }
            },
            WorkerInnerInput::Bus(BusEvent::Channel(owner, channel, event)) => match (owner, event)
            {
                (OwnerType::Whip(owner), SfuEvent::RequestKeyFrame(kind)) => {
                    self.switcher.flag_task(TaskType::Whip);
                    self.whip_group.input(&mut self.switcher).on_event(
                        now,
                        owner.index(),
                        WhipInput::Bus { kind },
                    );
                }
                (OwnerType::Whep(owner), SfuEvent::Media(media)) => {
                    self.switcher.flag_task(TaskType::Whep);
                    self.whep_group.input(&mut self.switcher).on_event(
                        now,
                        owner.index(),
                        WhepInput::Bus { channel, media },
                    );
                }
                _ => panic!("should not happen {:?}", owner),
            },
            _ => {}
        }
    }

    fn pop_output(
        &mut self,
        now: Instant,
    ) -> Option<WorkerInnerOutput<OwnerType, ExtOut, ChannelId, SfuEvent, SCfg>> {
        return_if_some!(self.output.pop_front());

        while let Some(current) = self.switcher.current() {
            match current.try_into().ok()? {
                TaskType::Whip => {
                    if let Some((index, out)) = self.whip_group.pop_output(now, &mut self.switcher)
                    {
                        return Some(self.process_whip_out(now, index, out));
                    }
                }
                TaskType::Whep => {
                    if let Some((index, out)) = self.whep_group.pop_output(now, &mut self.switcher)
                    {
                        return Some(self.process_whep_out(now, index, out));
                    }
                }
            }
        }
        None
    }

    fn on_shutdown(&mut self, now: Instant) {
        self.whip_group.input(&mut self.switcher).on_shutdown(now);
        self.whep_group.input(&mut self.switcher).on_shutdown(now);
    }
}

impl TrackMedia {
    pub fn from_raw(rtp: RtpPacket) -> Self {
        Self {
            seq_no: rtp.seq_no,
            header: rtp.header,
            payload: rtp.payload,
            timestamp: rtp.timestamp,
        }
    }
}
