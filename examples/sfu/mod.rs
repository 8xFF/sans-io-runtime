use std::{
    collections::{hash_map::DefaultHasher, HashMap, VecDeque},
    hash::{Hash, Hasher},
    net::SocketAddr,
    time::Instant,
};

use derive_more::Display;
use sans_io_runtime::{
    NetIncoming, NetOutgoing, Owner, Task, TaskGroup, TaskGroupInput, TaskGroupOutput,
    TaskGroupOutputsState, TaskInput, TaskOutput, WorkerInner, WorkerInnerInput, WorkerInnerOutput,
};
use str0m::{
    change::DtlsCert,
    media::{KeyframeRequestKind, MediaTime},
    rtp::{RtpHeader, RtpPacket, SeqNo},
};

use crate::http::{HttpRequest, HttpResponse};

use self::{shared_port::SharedUdpPort, whep::WhepTask, whip::WhipTask};

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

#[derive(Clone, Debug)]
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

pub struct SfuWorker {
    worker: u16,
    dtls_cert: DtlsCert,
    whip_group: TaskGroup<ChannelId, SfuEvent, WhipTask, 128>,
    whep_group: TaskGroup<ChannelId, SfuEvent, WhepTask, 128>,
    output: VecDeque<WorkerInnerOutput<'static, ExtOut, ChannelId, SfuEvent>>,
    shared_udp: SharedUdpPort<TaskId>,
    last_input: Option<u16>,
    groups_output: TaskGroupOutputsState<2>,
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
            self.shared_udp.get_addr().expect(""),
            self.dtls_cert.clone(),
            channel,
            &String::from_utf8_lossy(&req.body),
        );
        match task {
            Ok(task) => {
                log::info!("Whip enpoint created {}", task.ice_ufrag);
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
            self.shared_udp.get_addr().expect(""),
            self.dtls_cert.clone(),
            channel,
            &String::from_utf8_lossy(&req.body),
        );
        match task {
            Ok(task) => {
                log::info!("Whep enpoint created {}", task.ice_ufrag);
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

impl WorkerInner<ExtIn, ExtOut, ChannelId, SfuEvent, ICfg, SCfg> for SfuWorker {
    fn build(worker: u16, cfg: ICfg) -> Self {
        Self {
            worker,
            dtls_cert: DtlsCert::new(),
            whip_group: TaskGroup::new(worker),
            whep_group: TaskGroup::new(worker),
            output: VecDeque::from([WorkerInnerOutput::Task(
                Owner::worker(worker),
                TaskOutput::Net(NetOutgoing::UdpListen(cfg.udp_addr)),
            )]),
            shared_udp: SharedUdpPort::default(),
            last_input: None,
            groups_output: TaskGroupOutputsState::default(),
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
    fn on_input_tick<'a>(
        &mut self,
        now: Instant,
    ) -> Option<WorkerInnerOutput<'a, ExtOut, ChannelId, SfuEvent>> {
        loop {
            match self.groups_output.current()? {
                WhipTask::TYPE => match self.whip_group.on_input_tick(now) {
                    Some(res) => {
                        if matches!(res.1, TaskOutput::Destroy) {
                            self.shared_udp.remove_task(TaskId::Whip(
                                res.0.task_index().expect("Should have task"),
                            ));
                        }
                        return Some(res.into());
                    }
                    None => {
                        self.groups_output.finish_current();
                    }
                },
                WhepTask::TYPE => match self.whep_group.on_input_tick(now) {
                    Some(res) => {
                        if matches!(res.1, TaskOutput::Destroy) {
                            self.shared_udp.remove_task(TaskId::Whep(
                                res.0.task_index().expect("Should have task"),
                            ));
                        }
                        return Some(res.into());
                    }
                    None => {
                        self.groups_output.finish_current();
                    }
                },
                _ => panic!("Unknown task type"),
            }
        }
    }

    fn on_input_event<'a>(
        &mut self,
        now: Instant,
        event: WorkerInnerInput<'a, ExtIn, ChannelId, SfuEvent>,
    ) -> Option<WorkerInnerOutput<'a, ExtOut, ChannelId, SfuEvent>> {
        match event {
            WorkerInnerInput::Task(owner, event) => match event {
                TaskInput::Net(NetIncoming::UdpListenResult { bind: _, result }) => {
                    let addr = result.as_ref().expect("Should listen shared port ok");
                    self.shared_udp.set_addr(*addr);
                    self.last_input = None;
                    None
                }
                TaskInput::Net(NetIncoming::UdpPacket { from, to, data }) => {
                    match self.shared_udp.map_remote(from, data) {
                        Some(TaskId::Whip(index)) => {
                            self.last_input = Some(WhipTask::TYPE);
                            let owner = Owner::task(self.worker, WhipTask::TYPE, index);
                            let TaskGroupOutput(owner, output) = self.whip_group.on_input_event(
                                now,
                                TaskGroupInput(
                                    owner,
                                    TaskInput::Net(NetIncoming::UdpPacket { from, to, data }),
                                ),
                            )?;
                            Some(WorkerInnerOutput::Task(owner, output))
                        }
                        Some(TaskId::Whep(index)) => {
                            self.last_input = Some(WhepTask::TYPE);
                            let owner = Owner::task(self.worker, WhepTask::TYPE, index);
                            let TaskGroupOutput(owner, output) = self.whep_group.on_input_event(
                                now,
                                TaskGroupInput(
                                    owner,
                                    TaskInput::Net(NetIncoming::UdpPacket { from, to, data }),
                                ),
                            )?;
                            Some(WorkerInnerOutput::Task(owner, output))
                        }
                        None => {
                            log::warn!("Unknown remote address: {}", from);
                            None
                        }
                    }
                }
                TaskInput::Bus(channel, event) => {
                    self.last_input = owner.group_id();
                    match owner.group_id() {
                        Some(WhipTask::TYPE) => {
                            let TaskGroupOutput(owner, output) = self.whip_group.on_input_event(
                                now,
                                TaskGroupInput(owner, TaskInput::Bus(channel, event)),
                            )?;
                            Some(WorkerInnerOutput::Task(owner, output))
                        }
                        Some(WhepTask::TYPE) => {
                            let TaskGroupOutput(owner, output) = self.whep_group.on_input_event(
                                now,
                                TaskGroupInput(owner, TaskInput::Bus(channel, event)),
                            )?;
                            Some(WorkerInnerOutput::Task(owner, output))
                        }
                        _ => None,
                    }
                }
            },
            WorkerInnerInput::Ext(_ext) => {
                self.last_input = None;
                None
            }
        }
    }

    fn pop_last_input<'a>(
        &mut self,
        now: Instant,
    ) -> Option<WorkerInnerOutput<'a, ExtOut, ChannelId, SfuEvent>> {
        match self.last_input? {
            WhipTask::TYPE => self.whip_group.pop_last_input(now).map(|o| o.into()),
            WhepTask::TYPE => self.whep_group.pop_last_input(now).map(|o| o.into()),
            _ => None,
        }
    }

    fn pop_output<'a>(
        &mut self,
        now: Instant,
    ) -> Option<WorkerInnerOutput<'a, ExtOut, ChannelId, SfuEvent>> {
        if let Some(e) = self.output.pop_front() {
            return Some(e);
        }

        loop {
            match self.groups_output.current()? {
                WhipTask::TYPE => match self.whip_group.pop_output(now) {
                    Some(res) => {
                        if matches!(res.1, TaskOutput::Destroy) {
                            self.shared_udp.remove_task(TaskId::Whip(
                                res.0.task_index().expect("Should have task"),
                            ));
                        }
                        return Some(res.into());
                    }
                    None => {
                        self.groups_output.finish_current();
                    }
                },
                WhepTask::TYPE => match self.whep_group.pop_output(now) {
                    Some(res) => {
                        if matches!(res.1, TaskOutput::Destroy) {
                            self.shared_udp.remove_task(TaskId::Whep(
                                res.0.task_index().expect("Should have task"),
                            ));
                        }
                        return Some(res.into());
                    }
                    None => {
                        self.groups_output.finish_current();
                    }
                },
                _ => panic!("Unknown task type"),
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
