/// A user session with boolean-soup state tracking.
///
/// 7 booleans + 3 Options = 2^10 = 1,024 representable states.
/// Can a session be both expired and active? Locked out but verified?
/// The runtime checks catch some of these, but the type system allows all of them.
pub struct Session {
    pub is_active: bool,
    pub is_expired: bool,
    pub is_locked: bool,
    pub email_verified: bool,
    pub mfa_verified: bool,
    pub is_admin: bool,
    pub remember_me: bool,
    pub user_id: Option<u64>,
    pub csrf_token: Option<String>,
    pub expires_at: Option<u64>,
}

impl Session {
    pub fn new() -> Self {
        Self {
            is_active: false,
            is_expired: false,
            is_locked: false,
            email_verified: false,
            mfa_verified: false,
            is_admin: false,
            remember_me: false,
            user_id: None,
            csrf_token: None,
            expires_at: None,
        }
    }

    pub fn login(&mut self, user_id: u64, is_admin: bool) -> Result<(), String> {
        if self.is_active {
            return Err("already logged in".into());
        }
        if self.is_locked {
            return Err("account is locked".into());
        }
        self.user_id = Some(user_id);
        self.is_active = true;
        self.is_admin = is_admin;
        self.csrf_token = Some(format!("csrf-{user_id}"));
        self.expires_at = Some(now_epoch() + 3600);
        Ok(())
    }

    pub fn verify_email(&mut self) -> Result<(), String> {
        if !self.is_active {
            return Err("not logged in".into());
        }
        self.email_verified = true;
        Ok(())
    }

    pub fn verify_mfa(&mut self, _code: &str) -> Result<(), String> {
        if !self.is_active {
            return Err("not logged in".into());
        }
        if !self.email_verified {
            return Err("email not verified".into());
        }
        self.mfa_verified = true;
        Ok(())
    }

    pub fn check_permission(&self, _resource: &str) -> bool {
        self.is_active
            && !self.is_expired
            && !self.is_locked
            && self.email_verified
            && self.user_id.is_some()
    }

    pub fn check_admin_permission(&self, _resource: &str) -> bool {
        self.check_permission(_resource) && self.is_admin && self.mfa_verified
    }

    pub fn refresh(&mut self) -> Result<(), String> {
        if !self.is_active || self.is_expired {
            return Err("session not active".into());
        }
        self.expires_at = Some(now_epoch() + 3600);
        Ok(())
    }

    pub fn expire(&mut self) {
        self.is_expired = true;
        self.is_active = false;
    }

    pub fn lock(&mut self) {
        self.is_locked = true;
    }

    pub fn logout(&mut self) {
        self.is_active = false;
        self.is_expired = false;
        self.is_locked = false;
        self.email_verified = false;
        self.mfa_verified = false;
        self.is_admin = false;
        self.user_id = None;
        self.csrf_token = None;
        self.expires_at = None;
    }

    pub fn is_fully_authenticated(&self) -> bool {
        self.is_active
            && !self.is_expired
            && !self.is_locked
            && self.email_verified
            && self.mfa_verified
            && self.user_id.is_some()
            && self.csrf_token.is_some()
    }
}

fn now_epoch() -> u64 {
    0
}
