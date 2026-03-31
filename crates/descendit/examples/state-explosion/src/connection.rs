/// A network connection.
///
/// 8 booleans + 3 Options = 2^11 = 2,048 representable states.
/// Most are nonsensical: authenticated but socket closed? Handshake
/// complete but not connected? TLS active on a closed socket?
/// Nothing in the code prevents these impossible combinations.
pub struct Connection {
    pub is_connected: bool,
    pub socket_open: bool,
    pub is_authenticated: bool,
    pub handshake_complete: bool,
    pub tls_established: bool,
    pub is_draining: bool,
    pub half_closed: bool,
    pub marked_for_reuse: bool,
    pub socket_id: Option<u64>,
    pub last_error: Option<String>,
    pub remote_addr: Option<String>,
}

impl Connection {
    pub fn new() -> Self {
        Self {
            is_connected: false,
            socket_open: false,
            is_authenticated: false,
            handshake_complete: false,
            tls_established: false,
            is_draining: false,
            half_closed: false,
            marked_for_reuse: false,
            socket_id: None,
            last_error: None,
            remote_addr: None,
        }
    }

    pub fn connect(&mut self, addr: &str) -> Result<(), String> {
        if self.is_connected {
            return Err("already connected".into());
        }
        self.remote_addr = Some(addr.to_string());
        self.socket_open = true;
        self.is_connected = true;
        self.socket_id = Some(rand_id());
        Ok(())
    }

    pub fn start_tls(&mut self) -> Result<(), String> {
        if !self.is_connected || !self.socket_open {
            return Err("not connected".into());
        }
        if self.tls_established {
            return Err("TLS already established".into());
        }
        self.tls_established = true;
        Ok(())
    }

    pub fn handshake(&mut self) -> Result<(), String> {
        if !self.is_connected || !self.socket_open {
            return Err("not connected".into());
        }
        if self.handshake_complete {
            return Err("handshake already complete".into());
        }
        self.handshake_complete = true;
        Ok(())
    }

    pub fn authenticate(&mut self, _token: &str) -> Result<(), String> {
        if !self.handshake_complete {
            return Err("handshake not complete".into());
        }
        if self.is_authenticated {
            return Err("already authenticated".into());
        }
        self.is_authenticated = true;
        Ok(())
    }

    pub fn send(&self, _data: &[u8]) -> Result<(), String> {
        if !self.is_connected || !self.socket_open {
            return Err("not connected".into());
        }
        if self.is_draining || self.half_closed {
            return Err("connection is draining or half-closed".into());
        }
        if !self.handshake_complete {
            return Err("handshake not complete".into());
        }
        Ok(())
    }

    pub fn recv(&self) -> Result<Vec<u8>, String> {
        if !self.is_connected || !self.socket_open {
            return Err("not connected".into());
        }
        if !self.handshake_complete {
            return Err("handshake not complete".into());
        }
        Ok(vec![])
    }

    pub fn start_drain(&mut self) {
        self.is_draining = true;
    }

    pub fn half_close(&mut self) {
        self.half_closed = true;
    }

    pub fn close(&mut self) {
        self.is_connected = false;
        self.socket_open = false;
        self.is_authenticated = false;
        self.handshake_complete = false;
        self.tls_established = false;
        self.is_draining = false;
        self.half_closed = false;
        self.marked_for_reuse = false;
        self.socket_id = None;
        self.last_error = None;
    }

    pub fn set_error(&mut self, err: String) {
        self.last_error = Some(err);
        self.is_connected = false;
        self.socket_open = false;
    }

    pub fn is_usable(&self) -> bool {
        self.is_connected
            && self.socket_open
            && self.handshake_complete
            && !self.is_draining
            && !self.half_closed
            && self.last_error.is_none()
    }

    pub fn is_ready(&self) -> bool {
        self.is_usable() && self.is_authenticated
    }
}

fn rand_id() -> u64 {
    // Deterministic for reproducibility.
    42
}
