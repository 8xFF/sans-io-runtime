use std::{collections::HashMap, fmt::Debug, hash::Hash, net::SocketAddr};
use str0m::ice::StunMessage;

#[derive(Debug)]
pub struct SharedUdpPort<Task> {
    backend_addr: Option<SocketAddr>,
    backend_slot: Option<usize>,
    task_remotes: HashMap<SocketAddr, Task>,
    task_remotes_map: HashMap<Task, Vec<SocketAddr>>,
    task_ufrags: HashMap<String, Task>,
    task_ufrags_reverse: HashMap<Task, String>,
}

impl<Task> Default for SharedUdpPort<Task> {
    fn default() -> Self {
        Self {
            backend_addr: None,
            backend_slot: None,
            task_remotes: HashMap::new(),
            task_remotes_map: HashMap::new(),
            task_ufrags: HashMap::new(),
            task_ufrags_reverse: HashMap::new(),
        }
    }
}

impl<Task: Debug + Clone + Copy + Hash + PartialEq + Eq> SharedUdpPort<Task> {
    pub fn set_backend_info(&mut self, addr: SocketAddr, slot: usize) {
        self.backend_addr = Some(addr);
        self.backend_slot = Some(slot);
    }

    pub fn get_backend_addr(&self) -> Option<SocketAddr> {
        self.backend_addr
    }

    pub fn get_backend_slot(&self) -> Option<usize> {
        self.backend_slot
    }

    pub fn add_ufrag(&mut self, ufrag: String, task: Task) {
        log::info!("Add ufrag {} to task {:?}", ufrag, task);
        self.task_ufrags.insert(ufrag.clone(), task);
        self.task_ufrags_reverse.insert(task, ufrag);
    }

    pub fn remove_task(&mut self, task: Task) -> Option<()> {
        let ufrag = self.task_ufrags_reverse.remove(&task)?;
        log::info!("Remove task {:?} => ufrag {}", task, ufrag);
        self.task_ufrags.remove(&ufrag)?;
        let remotes = self.task_remotes_map.remove(&task)?;
        for remote in remotes {
            log::info!("     Remove remote {:?} => task {:?}", remote, task);
            self.task_remotes.remove(&remote);
        }
        Some(())
    }

    pub fn map_remote(&mut self, remote: SocketAddr, buf: &[u8]) -> Option<Task> {
        if let Some(task) = self.task_remotes.get(&remote) {
            return Some(*task);
        }

        let msg = StunMessage::parse(buf).ok()?;
        let (stun_username, _other) = msg.split_username()?;
        log::warn!(
            "Received a stun packet from an unknown remote: {:?}, username {}",
            remote,
            stun_username
        );
        let task = self.task_ufrags.get(stun_username)?;
        log::info!("Mapping remote {:?} to task {:?}", remote, task);
        self.task_remotes.insert(remote, *task);
        self.task_remotes_map.entry(*task).or_default().push(remote);
        Some(*task)
    }
}
