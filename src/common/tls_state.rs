#[derive(Debug, Copy, Clone)]
pub(crate) enum TlsState {
    #[cfg(feature = "early-data")]
    EarlyData,
    Stream,
    ReadShutdown,
    WriteShutdown,
    FullyShutdown,
}

impl TlsState {
    pub(crate) fn shutdown_read(&mut self) {
        match *self {
            TlsState::WriteShutdown | TlsState::FullyShutdown => *self = TlsState::FullyShutdown,
            _ => *self = TlsState::ReadShutdown,
        }
    }

    pub(crate) fn shutdown_write(&mut self) {
        match *self {
            TlsState::ReadShutdown | TlsState::FullyShutdown => *self = TlsState::FullyShutdown,
            _ => *self = TlsState::WriteShutdown,
        }
    }

    pub(crate) fn writeable(&self) -> bool {
        !matches!(*self, TlsState::WriteShutdown | TlsState::FullyShutdown)
    }

    pub(crate) fn readable(self) -> bool {
        !matches!(self, TlsState::ReadShutdown | TlsState::FullyShutdown)
    }
}
