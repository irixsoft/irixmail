use tokio::sync::watch;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShutdownCause {
    Terminate,
    Interrupt,
    Internal,
}

impl ShutdownCause {
    pub fn label(self) -> &'static str {
        match self {
            ShutdownCause::Terminate => "sigterm",
            ShutdownCause::Interrupt => "sigint",
            ShutdownCause::Internal => "internal",
        }
    }
}

pub struct Shutdown {
    tx: watch::Sender<bool>,
}

impl Shutdown {
    pub fn new() -> Self {
        let (tx, _rx) = watch::channel(false);
        Self { tx }
    }

    pub fn subscribe(&self) -> ShutdownSignal {
        ShutdownSignal {
            rx: self.tx.subscribe(),
        }
    }

    pub fn is_triggered(&self) -> bool {
        *self.tx.borrow()
    }

    pub fn trigger(&self, cause: ShutdownCause) {
        if self.is_triggered() {
            tracing::debug!(cause = cause.label(), "shutdown already in progress");
            return;
        }
        tracing::info!(cause = cause.label(), "broadcasting graceful shutdown");
        // send_replace updates the value even with no current receiver, so a task
        // that subscribes later still observes the stop.
        self.tx.send_replace(true);
    }

    pub async fn wait_for_signal() -> ShutdownCause {
        wait_for_signal().await
    }
}

impl Default for Shutdown {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone)]
pub struct ShutdownSignal {
    rx: watch::Receiver<bool>,
}

impl ShutdownSignal {
    pub async fn recv(&mut self) {
        if *self.rx.borrow() {
            return;
        }
        let _ = self.rx.changed().await;
    }

    pub fn is_triggered(&self) -> bool {
        *self.rx.borrow()
    }
}

#[cfg(unix)]
async fn wait_for_signal() -> ShutdownCause {
    use tokio::signal::unix::{signal, SignalKind};

    let mut terminate = match signal(SignalKind::terminate()) {
        Ok(stream) => stream,
        Err(error) => {
            tracing::error!(%error, "could not install SIGTERM handler; awaiting Ctrl-C only");
            let _ = tokio::signal::ctrl_c().await;
            return ShutdownCause::Interrupt;
        }
    };
    let mut interrupt = match signal(SignalKind::interrupt()) {
        Ok(stream) => stream,
        Err(error) => {
            tracing::error!(%error, "could not install SIGINT handler; awaiting SIGTERM only");
            terminate.recv().await;
            return ShutdownCause::Terminate;
        }
    };

    tokio::select! {
        _ = terminate.recv() => ShutdownCause::Terminate,
        _ = interrupt.recv() => ShutdownCause::Interrupt,
    }
}

#[cfg(not(unix))]
async fn wait_for_signal() -> ShutdownCause {
    let _ = tokio::signal::ctrl_c().await;
    ShutdownCause::Interrupt
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_coordinator_is_not_triggered() {
        let shutdown = Shutdown::new();
        assert!(!shutdown.is_triggered());
        assert!(!shutdown.subscribe().is_triggered());
    }

    #[test]
    fn the_cause_label_names_each_variant() {
        assert_eq!(ShutdownCause::Terminate.label(), "sigterm");
        assert_eq!(ShutdownCause::Interrupt.label(), "sigint");
        assert_eq!(ShutdownCause::Internal.label(), "internal");
    }

    #[tokio::test]
    async fn triggering_wakes_a_waiting_subscriber() {
        let shutdown = Shutdown::new();
        let mut signal = shutdown.subscribe();

        shutdown.trigger(ShutdownCause::Terminate);

        signal.recv().await;
        assert!(signal.is_triggered());
        assert!(shutdown.is_triggered());
    }

    #[tokio::test]
    async fn subscribing_after_a_trigger_still_observes_the_stop() {
        let shutdown = Shutdown::new();
        shutdown.trigger(ShutdownCause::Interrupt);

        let mut late = shutdown.subscribe();
        assert!(late.is_triggered());
        late.recv().await;
    }

    #[tokio::test]
    async fn one_trigger_reaches_every_subscriber() {
        let shutdown = Shutdown::new();
        let mut first = shutdown.subscribe();
        let mut second = shutdown.subscribe();

        shutdown.trigger(ShutdownCause::Internal);

        first.recv().await;
        second.recv().await;
        assert!(first.is_triggered());
        assert!(second.is_triggered());
    }

    #[tokio::test]
    async fn triggering_twice_is_harmless() {
        let shutdown = Shutdown::new();
        let mut signal = shutdown.subscribe();

        shutdown.trigger(ShutdownCause::Terminate);
        shutdown.trigger(ShutdownCause::Interrupt);

        signal.recv().await;
        assert!(shutdown.is_triggered());
    }

    #[tokio::test]
    async fn a_signal_selects_against_other_work() {
        let shutdown = Shutdown::new();
        let mut signal = shutdown.subscribe();
        shutdown.trigger(ShutdownCause::Terminate);

        let stopped = tokio::select! {
            _ = signal.recv() => true,
            _ = std::future::pending::<()>() => false,
        };
        assert!(stopped);
    }

    #[tokio::test]
    async fn a_dropped_coordinator_does_not_stall_a_subscriber() {
        let shutdown = Shutdown::new();
        let mut signal = shutdown.subscribe();
        drop(shutdown);

        signal.recv().await;
    }
}
