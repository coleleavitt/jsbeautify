#[derive(Debug)]
pub struct ReconBudget {
    payload_bytes_remaining: usize,
    exhausted: bool,
}

impl ReconBudget {
    pub const fn new(payload_bytes: usize) -> Self {
        Self {
            payload_bytes_remaining: payload_bytes,
            exhausted: false,
        }
    }

    pub const fn try_reserve(&mut self, bytes: usize) -> bool {
        if bytes > self.payload_bytes_remaining {
            self.exhausted = true;
            return false;
        }
        self.payload_bytes_remaining -= bytes;
        true
    }

    pub fn take_str(&mut self, value: &str) -> Option<String> {
        if value.len() > self.payload_bytes_remaining {
            self.exhausted = true;
            return None;
        }
        self.payload_bytes_remaining -= value.len();
        Some(value.to_owned())
    }

    pub const fn exhausted(&self) -> bool {
        self.exhausted
    }
}
