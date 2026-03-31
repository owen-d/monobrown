/// An HTTP-like request being assembled.
///
/// 6 booleans + 4 Options = 2^10 = 1,024 representable states.
/// Can a request be "sent" but also "pending"? Can it have a response
/// body but no status code? The booleans don't prevent it.
pub struct Request {
    pub method: String,
    pub url: String,
    pub headers_sent: bool,
    pub body_sent: bool,
    pub is_pending: bool,
    pub is_complete: bool,
    pub is_cancelled: bool,
    pub has_timed_out: bool,
    pub status_code: Option<u16>,
    pub response_body: Option<Vec<u8>>,
    pub error_message: Option<String>,
    pub redirect_url: Option<String>,
}

impl Request {
    pub fn new(method: &str, url: &str) -> Self {
        Self {
            method: method.to_string(),
            url: url.to_string(),
            headers_sent: false,
            body_sent: false,
            is_pending: false,
            is_complete: false,
            is_cancelled: false,
            has_timed_out: false,
            status_code: None,
            response_body: None,
            error_message: None,
            redirect_url: None,
        }
    }

    pub fn send_headers(&mut self) -> Result<(), String> {
        if self.headers_sent {
            return Err("headers already sent".into());
        }
        if self.is_complete || self.is_cancelled {
            return Err("request already finished".into());
        }
        self.headers_sent = true;
        self.is_pending = true;
        Ok(())
    }

    pub fn send_body(&mut self, _body: &[u8]) -> Result<(), String> {
        if !self.headers_sent {
            return Err("headers not sent".into());
        }
        if self.body_sent {
            return Err("body already sent".into());
        }
        if self.is_complete || self.is_cancelled {
            return Err("request already finished".into());
        }
        self.body_sent = true;
        Ok(())
    }

    pub fn receive_response(&mut self, status: u16, body: Vec<u8>) -> Result<(), String> {
        if !self.is_pending {
            return Err("request not pending".into());
        }
        if self.is_complete {
            return Err("already have response".into());
        }
        self.status_code = Some(status);
        self.response_body = Some(body);
        self.is_complete = true;
        self.is_pending = false;
        Ok(())
    }

    pub fn cancel(&mut self) {
        self.is_cancelled = true;
        self.is_pending = false;
    }

    pub fn timeout(&mut self) {
        self.has_timed_out = true;
        self.is_pending = false;
        self.error_message = Some("request timed out".into());
    }

    pub fn follow_redirect(&mut self) -> Result<(), String> {
        if !self.is_complete {
            return Err("no response to redirect from".into());
        }
        match self.status_code {
            Some(301 | 302 | 307 | 308) => {}
            _ => return Err("not a redirect status".into()),
        }
        let new_url = self
            .redirect_url
            .take()
            .ok_or("no redirect URL")?;
        self.url = new_url;
        self.headers_sent = false;
        self.body_sent = false;
        self.is_pending = false;
        self.is_complete = false;
        self.status_code = None;
        self.response_body = None;
        Ok(())
    }

    pub fn is_success(&self) -> bool {
        self.is_complete
            && !self.is_cancelled
            && !self.has_timed_out
            && self.error_message.is_none()
            && matches!(self.status_code, Some(200..=299))
    }
}
