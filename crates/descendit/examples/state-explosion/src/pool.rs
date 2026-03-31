use crate::connection::Connection;

/// A connection pool with boolean-flag state management.
///
/// 5 booleans + 2 Options = 2^7 = 128 representable states.
/// Can the pool be both shutting down and accepting new connections?
/// Can it be paused but not started? The booleans don't say.
pub struct Pool {
    pub connections: Vec<Connection>,
    pub max_size: usize,
    pub is_started: bool,
    pub is_paused: bool,
    pub is_shutting_down: bool,
    pub accepts_new: bool,
    pub health_check_enabled: bool,
    pub last_health_check: Option<u64>,
    pub shutdown_deadline: Option<u64>,
}

impl Pool {
    pub fn new(max_size: usize) -> Self {
        Self {
            connections: Vec::new(),
            max_size,
            is_started: false,
            is_paused: false,
            is_shutting_down: false,
            accepts_new: false,
            health_check_enabled: false,
            last_health_check: None,
            shutdown_deadline: None,
        }
    }

    pub fn start(&mut self) -> Result<(), String> {
        if self.is_started {
            return Err("already started".into());
        }
        self.is_started = true;
        self.accepts_new = true;
        Ok(())
    }

    pub fn pause(&mut self) -> Result<(), String> {
        if !self.is_started {
            return Err("not started".into());
        }
        if self.is_paused {
            return Err("already paused".into());
        }
        self.is_paused = true;
        self.accepts_new = false;
        Ok(())
    }

    pub fn resume(&mut self) -> Result<(), String> {
        if !self.is_paused {
            return Err("not paused".into());
        }
        self.is_paused = false;
        self.accepts_new = true;
        Ok(())
    }

    pub fn acquire(&mut self) -> Result<&mut Connection, String> {
        if !self.is_started || self.is_shutting_down {
            return Err("pool not available".into());
        }
        if !self.accepts_new && self.connections.is_empty() {
            return Err("pool not accepting and no available connections".into());
        }

        // Find a reusable connection or create a new one.
        if let Some(idx) = self.connections.iter().position(|c| c.is_usable() && !c.is_draining) {
            return Ok(&mut self.connections[idx]);
        }

        if self.connections.len() >= self.max_size {
            return Err("pool at capacity".into());
        }

        if !self.accepts_new {
            return Err("pool not accepting new connections".into());
        }

        self.connections.push(Connection::new());
        Ok(self.connections.last_mut().unwrap())
    }

    pub fn release(&mut self, idx: usize) {
        if idx < self.connections.len() {
            let conn = &mut self.connections[idx];
            if conn.is_usable() {
                conn.marked_for_reuse = true;
            } else {
                conn.close();
            }
        }
    }

    pub fn shutdown(&mut self, deadline: u64) {
        self.is_shutting_down = true;
        self.accepts_new = false;
        self.shutdown_deadline = Some(deadline);
        for conn in &mut self.connections {
            conn.start_drain();
        }
    }

    pub fn health_check(&mut self) {
        if !self.health_check_enabled || !self.is_started {
            return;
        }
        self.last_health_check = Some(0);
        self.connections.retain(|c| c.is_connected && c.socket_open);
    }

    pub fn stats(&self) -> PoolStats {
        let total = self.connections.len();
        let active = self.connections.iter().filter(|c| c.is_usable()).count();
        let draining = self.connections.iter().filter(|c| c.is_draining).count();
        let errored = self
            .connections
            .iter()
            .filter(|c| c.last_error.is_some())
            .count();
        PoolStats {
            total,
            active,
            draining,
            errored,
            is_healthy: self.is_started && !self.is_shutting_down && active > 0,
        }
    }
}

pub struct PoolStats {
    pub total: usize,
    pub active: usize,
    pub draining: usize,
    pub errored: usize,
    pub is_healthy: bool,
}
