use std::{
    collections::{hash_map::DefaultHasher, HashMap, VecDeque},
    hash::{Hash, Hasher},
    net::SocketAddr,
    time::Instant,
};

use derive_more::Display;
use sans_io_runtime::{
    NetIncoming, NetOutgoing, Owner, Task, TaskGroup, TaskGroupOutput, WorkerCtx, WorkerInner,
    WorkerInnerOutput,
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

#[derive(Display, Clone, Copy, Hash, PartialEq, Eq)]
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
    whip_group: TaskGroup<ChannelId, SfuEvent, WhipTask, 128, 1024>,
    whep_group: TaskGroup<ChannelId, SfuEvent, WhepTask, 128, 1024>,
    output: VecDeque<WorkerInnerOutput<'static, ExtOut, ChannelId, SfuEvent>>,
    shared_udp: SharedUdpPort<TaskId>,
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
            output: VecDeque::from([WorkerInnerOutput::Net(
                Owner::worker(worker),
                NetOutgoing::UdpListen(cfg.udp_addr),
            )]),
            shared_udp: SharedUdpPort::default(),
        }
    }
    fn worker_index(&self) -> u16 {
        self.worker
    }
    fn tasks(&self) -> usize {
        self.whip_group.tasks() + self.whep_group.tasks()
    }
    fn spawn(&mut self, _now: Instant, _ctx: &mut WorkerCtx<'_>, cfg: SCfg) {
        match cfg {
            SCfg::HttpRequest(req) => {
                self.process_req(req);
            }
        }
    }
    fn on_ext(&mut self, _now: Instant, _ctx: &mut WorkerCtx<'_>, _ext: ExtIn) {
        // self.whip_group.on_ext(now, ctx, ext);
        // self.whep_group.on_ext(now, ctx, ext);
    }
    fn on_bus(
        &mut self,
        now: Instant,
        ctx: &mut WorkerCtx<'_>,
        owner: Owner,
        channel_id: ChannelId,
        event: SfuEvent,
    ) {
        match owner.group_id() {
            Some(1) => {
                self.whip_group.on_bus(now, ctx, owner, channel_id, event);
            }
            Some(2) => {
                self.whep_group.on_bus(now, ctx, owner, channel_id, event);
            }
            _ => unreachable!(),
        }
    }
    fn on_net(&mut self, now: Instant, _owner: Owner, net: NetIncoming) {
        match &net {
            NetIncoming::UdpListenResult { bind: _, result } => {
                let addr = result.as_ref().expect("Should listen shared port ok");
                self.shared_udp.set_addr(*addr);
            }
            NetIncoming::UdpPacket { from, to: _, data } => {
                match self.shared_udp.map_remote(*from, data) {
                    Some(TaskId::Whip(index)) => {
                        self.whip_group.on_net(
                            now,
                            Owner::task(self.worker, WhipTask::TYPE, index),
                            net,
                        );
                    }
                    Some(TaskId::Whep(index)) => {
                        self.whep_group.on_net(
                            now,
                            Owner::task(self.worker, WhepTask::TYPE, index),
                            net,
                        );
                    }
                    None => {
                        log::warn!("Unknown remote address: {}", from);
                    }
                }
            }
        }
    }

    fn inner_process(&mut self, now: Instant, ctx: &mut WorkerCtx<'_>) {
        self.whip_group.on_tick(now, ctx);
        self.whep_group.on_tick(now, ctx);
    }

    fn pop_output(&mut self) -> Option<WorkerInnerOutput<'_, ExtOut, ChannelId, SfuEvent>> {
        while let Some(event) = self.whip_group.pop_output() {
            match event {
                TaskGroupOutput::Bus(owner, event) => {
                    self.output.push_back(WorkerInnerOutput::Bus(owner, event));
                }
                TaskGroupOutput::DestroyOwner(owner) => {
                    self.shared_udp
                        .remove_task(TaskId::Whip(owner.task_index().expect("Should have task")));
                }
            }
        }

        while let Some(event) = self.whep_group.pop_output() {
            match event {
                TaskGroupOutput::Bus(owner, event) => {
                    self.output.push_back(WorkerInnerOutput::Bus(owner, event));
                }
                TaskGroupOutput::DestroyOwner(owner) => {
                    self.shared_udp
                        .remove_task(TaskId::Whip(owner.task_index().expect("Should have task")));
                }
            }
        }

        self.output.pop_front()
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
